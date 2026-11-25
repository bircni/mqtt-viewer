//! Optional on-disk message history (`SQLite`).
//!
//! Off by default. When the user enables it, received messages are written through a [`StoreWriter`] thread so
//! disk latency never stalls the UI, and the most recent messages per topic are loaded back on connect.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use bytes::Bytes;
use jiff::Timestamp;
use rusqlite::{Connection, params};

use crate::message::{Message, Properties, QoS};

/// Errors from the history database.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The database directory could not be created.
    #[error("history database {path}: {source}")]
    Io {
        /// Database file.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The database reported an error.
    #[error("history database: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// What a retention pass should keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    /// Drop messages older than this many days; 0 keeps any age.
    pub max_age_days: u32,
    /// Drop the oldest messages until the stored payload data fits into this many bytes; 0 keeps any size.
    pub max_bytes: u64,
}

/// The message history database.
#[derive(Debug)]
pub struct Store {
    connection: Connection,
}

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS messages (
        id INTEGER PRIMARY KEY,
        profile TEXT NOT NULL,
        topic TEXT NOT NULL,
        received_ns INTEGER NOT NULL,
        payload BLOB NOT NULL,
        qos INTEGER NOT NULL,
        retain INTEGER NOT NULL,
        properties TEXT
    );
    CREATE INDEX IF NOT EXISTS messages_by_topic ON messages (profile, topic, id);
    CREATE INDEX IF NOT EXISTS messages_by_time ON messages (received_ns);
";

/// Approximate bytes a row occupies, used for the size limit.
const ROW_SIZE: &str = "LENGTH(payload) + LENGTH(topic) + LENGTH(profile) + IFNULL(LENGTH(properties), 0) + 40";

impl Store {
    /// Opens (creating if necessary) the database at `path`.
    ///
    /// # Errors
    /// Fails when the directory cannot be created or the file is not a usable database.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| StoreError::Io { path: path.to_owned(), source })?;
        }
        Self::initialize(Connection::open(path)?)
    }

    /// Opens a database that lives in memory only.
    ///
    /// # Errors
    /// Fails when the schema cannot be created.
    pub fn in_memory() -> Result<Self, StoreError> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(connection: Connection) -> Result<Self, StoreError> {
        // WAL keeps readers and the writer from blocking each other; it is a no-op for in-memory databases.
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.execute_batch(SCHEMA)?;
        Ok(Self { connection })
    }

    /// Stores `messages` for `profile` in one transaction.
    ///
    /// # Errors
    /// Fails when the database cannot be written.
    pub fn insert(&mut self, profile: &str, messages: &[Message]) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        {
            let mut statement = transaction.prepare_cached(
                "INSERT INTO messages (profile, topic, received_ns, payload, qos, retain, properties)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for message in messages {
                let properties =
                    message.properties.as_deref().and_then(|properties| serde_json::to_string(properties).ok());
                statement.execute(params![
                    profile,
                    message.topic,
                    nanoseconds(message.received_at),
                    &message.payload[..],
                    message.qos.level(),
                    message.retain,
                    properties,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// The newest `per_topic` messages of every topic of `profile`, oldest first.
    ///
    /// # Errors
    /// Fails when the database cannot be read.
    pub fn load_recent(&self, profile: &str, per_topic: usize) -> Result<Vec<Message>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT topic, received_ns, payload, qos, retain, properties FROM (
                 SELECT *, ROW_NUMBER() OVER (PARTITION BY topic ORDER BY id DESC) AS position
                 FROM messages WHERE profile = ?1
             ) WHERE position <= ?2 ORDER BY id",
        )?;
        let limit = i64::try_from(per_topic).unwrap_or(i64::MAX);
        let rows = statement.query_map(params![profile, limit], |row| {
            let received_ns: i64 = row.get(1)?;
            let payload: Vec<u8> = row.get(2)?;
            let qos: u8 = row.get(3)?;
            let properties: Option<String> = row.get(5)?;
            Ok(Message {
                topic: row.get(0)?,
                received_at: Timestamp::from_nanosecond(i128::from(received_ns)).unwrap_or(Timestamp::UNIX_EPOCH),
                payload: Bytes::from(payload),
                qos: QoS::try_from(qos).unwrap_or_default(),
                retain: row.get(4)?,
                properties: properties
                    .and_then(|text| serde_json::from_str::<Properties>(&text).ok())
                    .filter(|properties| !properties.is_empty())
                    .map(Box::new),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Applies the retention limits as of `now` and returns the number of deleted messages.
    ///
    /// # Errors
    /// Fails when the database cannot be written.
    pub fn prune(&mut self, retention: Retention, now: Timestamp) -> Result<usize, StoreError> {
        let mut deleted = 0;
        if retention.max_age_days > 0 {
            let age_ns = i64::from(retention.max_age_days) * 86_400 * 1_000_000_000;
            let cutoff = nanoseconds(now).saturating_sub(age_ns);
            deleted += self.connection.execute("DELETE FROM messages WHERE received_ns < ?1", params![cutoff])?;
        }
        if retention.max_bytes > 0 {
            let limit = i64::try_from(retention.max_bytes).unwrap_or(i64::MAX);
            // Find the oldest row that still fits when summing sizes from the newest row backwards.
            let boundary: Option<i64> = self.connection.query_row(
                &format!(
                    "SELECT MAX(id) FROM (
                         SELECT id, SUM({ROW_SIZE}) OVER (ORDER BY id DESC) AS running FROM messages
                     ) WHERE running > ?1"
                ),
                params![limit],
                |row| row.get(0),
            )?;
            if let Some(boundary) = boundary {
                deleted += self.connection.execute("DELETE FROM messages WHERE id <= ?1", params![boundary])?;
            }
        }
        Ok(deleted)
    }

    /// Deletes the stored messages of `profile`, or of every profile when `None`.
    ///
    /// # Errors
    /// Fails when the database cannot be written.
    pub fn clear(&mut self, profile: Option<&str>) -> Result<usize, StoreError> {
        let deleted = match profile {
            Some(profile) => self.connection.execute("DELETE FROM messages WHERE profile = ?1", params![profile])?,
            None => self.connection.execute("DELETE FROM messages", [])?,
        };
        Ok(deleted)
    }

    /// Number of stored messages, over all profiles.
    ///
    /// # Errors
    /// Fails when the database cannot be read.
    pub fn count(&self) -> Result<u64, StoreError> {
        let count: i64 = self.connection.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    /// Approximate bytes of stored message data, over all profiles.
    ///
    /// # Errors
    /// Fails when the database cannot be read.
    pub fn stored_bytes(&self) -> Result<u64, StoreError> {
        let bytes: i64 =
            self.connection
                .query_row(&format!("SELECT IFNULL(SUM({ROW_SIZE}), 0) FROM messages"), [], |row| row.get(0))?;
        Ok(u64::try_from(bytes).unwrap_or(0))
    }
}

fn nanoseconds(timestamp: Timestamp) -> i64 {
    i64::try_from(timestamp.as_nanosecond()).unwrap_or(i64::MAX)
}

enum Job {
    Insert { profile: String, messages: Vec<Message> },
    Prune(Retention),
    Clear(Option<String>),
    Flush(Sender<()>),
}

/// Writes to a [`Store`] on a background thread. Dropping it finishes pending work first.
#[derive(Debug)]
pub struct StoreWriter {
    jobs: Option<Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl StoreWriter {
    /// Moves `store` to a new writer thread.
    ///
    /// # Errors
    /// Fails when the thread cannot be started.
    pub fn spawn(store: Store) -> std::io::Result<Self> {
        let (jobs, queue) = channel();
        let thread =
            std::thread::Builder::new().name("mqtt-history".to_owned()).spawn(move || write_loop(store, &queue))?;
        Ok(Self { jobs: Some(jobs), thread: Some(thread) })
    }

    fn submit(&self, job: Job) {
        if self.jobs.as_ref().is_none_or(|jobs| jobs.send(job).is_err()) {
            log::warn!("history writer is gone; job dropped");
        }
    }

    /// Queues messages for storage.
    pub fn insert(&self, profile: &str, messages: Vec<Message>) {
        if !messages.is_empty() {
            self.submit(Job::Insert { profile: profile.to_owned(), messages });
        }
    }

    /// Queues a retention pass.
    pub fn prune(&self, retention: Retention) {
        self.submit(Job::Prune(retention));
    }

    /// Queues deletion of a profile's history, or of all history when `None`.
    pub fn clear(&self, profile: Option<&str>) {
        self.submit(Job::Clear(profile.map(str::to_owned)));
    }

    /// Blocks until everything queued so far has been written.
    pub fn flush(&self) {
        let (done, wait) = channel();
        self.submit(Job::Flush(done));
        let _ = wait.recv();
    }
}

impl Drop for StoreWriter {
    fn drop(&mut self) {
        // Closing the channel ends the loop once the queue is drained.
        self.jobs = None;
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            log::error!("history writer panicked");
        }
    }
}

fn write_loop(mut store: Store, queue: &Receiver<Job>) {
    for job in queue {
        let result = match job {
            Job::Insert { profile, messages } => store.insert(&profile, &messages),
            Job::Prune(retention) => store.prune(retention, Timestamp::now()).map(|_| ()),
            Job::Clear(profile) => store.clear(profile.as_deref()).map(|_| ()),
            Job::Flush(done) => {
                let _ = done.send(());
                Ok(())
            }
        };
        if let Err(error) = result {
            log::warn!("{error}");
        }
    }
}
