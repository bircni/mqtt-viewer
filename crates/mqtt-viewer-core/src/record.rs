//! Session recording, export and replay planning.
//!
//! A recording is a JSON Lines file: one [`RecordedMessage`] per line, payload Base64-encoded so binary data
//! survives. Exports (JSON, CSV) are meant for people and other tools and render text payloads as text.

use std::fs::File;
use std::io::{self, BufRead as _, BufReader, BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::message::{Message, Properties, PublishRequest, QoS, bytes_b64};
use crate::topic;

/// Errors from reading or writing recordings.
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    /// The file could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// File that failed.
        path: PathBuf,
        /// Underlying error.
        source: io::Error,
    },
    /// A line of the recording is not a valid message.
    #[error("{path}, line {line}: {source}")]
    Parse {
        /// File that failed.
        path: PathBuf,
        /// 1-based line number.
        line: usize,
        /// Underlying error.
        source: serde_json::Error,
    },
}

/// One message of a recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedMessage {
    /// When the message was received.
    pub ts: Timestamp,
    /// Topic.
    pub topic: String,
    /// Payload.
    #[serde(with = "bytes_b64")]
    pub payload: Bytes,
    /// QoS.
    #[serde(default)]
    pub qos: QoS,
    /// Retain flag.
    #[serde(default)]
    pub retain: bool,
    /// MQTT 5 properties.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<Properties>,
}

impl From<&Message> for RecordedMessage {
    fn from(message: &Message) -> Self {
        Self {
            ts: message.received_at,
            topic: message.topic.clone(),
            payload: message.payload.clone(),
            qos: message.qos,
            retain: message.retain,
            properties: message.properties.as_deref().cloned(),
        }
    }
}

/// Appends received messages to a recording file.
#[derive(Debug)]
pub struct Recorder {
    path: PathBuf,
    writer: BufWriter<File>,
    count: u64,
}

impl Recorder {
    /// Starts a new recording at `path`, replacing an existing file.
    ///
    /// # Errors
    /// Fails when the file cannot be created.
    pub fn create(path: &Path) -> Result<Self, RecordError> {
        let file = File::create(path).map_err(|source| RecordError::Io { path: path.to_owned(), source })?;
        Ok(Self { path: path.to_owned(), writer: BufWriter::new(file), count: 0 })
    }

    /// Where the recording is written.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of messages written so far.
    #[must_use]
    pub const fn count(&self) -> u64 {
        self.count
    }

    /// Appends one message.
    ///
    /// # Errors
    /// Fails when the file cannot be written.
    pub fn write(&mut self, message: &Message) -> Result<(), RecordError> {
        let line = serde_json::to_string(&RecordedMessage::from(message)).map_err(io::Error::other);
        line.and_then(|line| writeln!(self.writer, "{line}"))
            .map_err(|source| RecordError::Io { path: self.path.clone(), source })?;
        self.count += 1;
        Ok(())
    }

    /// Flushes and closes the recording, returning the number of messages written.
    ///
    /// # Errors
    /// Fails when buffered data cannot be written.
    pub fn finish(mut self) -> Result<u64, RecordError> {
        self.writer.flush().map_err(|source| RecordError::Io { path: self.path.clone(), source })?;
        Ok(self.count)
    }
}

/// Reads a recording written by [`Recorder`]. Blank lines are ignored.
///
/// # Errors
/// Fails when the file cannot be read or a line is malformed.
pub fn read_recording(path: &Path) -> Result<Vec<RecordedMessage>, RecordError> {
    let io_error = |source| RecordError::Io { path: path.to_owned(), source };
    let reader = BufReader::new(File::open(path).map_err(io_error)?);
    let mut messages = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line.map_err(io_error)?;
        if line.trim().is_empty() {
            continue;
        }
        let message = serde_json::from_str(&line).map_err(|source| RecordError::Parse {
            path: path.to_owned(),
            line: index + 1,
            source,
        })?;
        messages.push(message);
    }
    Ok(messages)
}

#[derive(Serialize)]
struct ExportRow<'a> {
    timestamp: Timestamp,
    topic: &'a str,
    qos: u8,
    retain: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload_base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    properties: Option<&'a Properties>,
}

/// Renders messages as a pretty-printed JSON array. Text payloads appear as `payload`, binary ones as
/// `payload_base64`.
#[must_use]
pub fn export_json(messages: &[RecordedMessage]) -> String {
    let rows: Vec<ExportRow<'_>> = messages
        .iter()
        .map(|message| {
            let text = std::str::from_utf8(&message.payload).ok();
            ExportRow {
                timestamp: message.ts,
                topic: &message.topic,
                qos: message.qos.level(),
                retain: message.retain,
                payload: text,
                payload_base64: text.is_none().then(|| STANDARD.encode(&message.payload)),
                properties: message.properties.as_ref(),
            }
        })
        .collect();
    serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_owned())
}

/// Renders messages as CSV with the columns `timestamp,topic,qos,retain,encoding,payload`. Binary payloads are
/// Base64-encoded and flagged in the `encoding` column.
#[must_use]
pub fn export_csv(messages: &[RecordedMessage]) -> String {
    let mut out = String::from("timestamp,topic,qos,retain,encoding,payload\n");
    for message in messages {
        let (encoding, payload) = std::str::from_utf8(&message.payload)
            .map_or_else(|_| ("base64", STANDARD.encode(&message.payload)), |text| ("text", text.to_owned()));
        let fields = [
            message.ts.to_string(),
            message.topic.clone(),
            message.qos.level().to_string(),
            message.retain.to_string(),
            encoding.to_owned(),
            payload,
        ];
        let line: Vec<String> = fields.iter().map(|field| csv_field(field)).collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

fn csv_field(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

/// How a recording is played back.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayOptions {
    /// Time scale: 1.0 keeps the original pacing, 2.0 is twice as fast, 0.0 sends everything at once.
    pub speed: f64,
    /// Topic prefix to replace; empty to keep topics unchanged.
    pub prefix_from: String,
    /// Replacement for `prefix_from`.
    pub prefix_to: String,
}

impl Default for ReplayOptions {
    fn default() -> Self {
        Self { speed: 1.0, prefix_from: String::new(), prefix_to: String::new() }
    }
}

/// One publish of a replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayStep {
    /// How long to wait after the previous step.
    pub delay: Duration,
    /// What to publish.
    pub request: PublishRequest,
}

/// Turns a recording into the sequence of publishes that replays it.
#[must_use]
pub fn plan_replay(messages: &[RecordedMessage], options: &ReplayOptions) -> Vec<ReplayStep> {
    let mut previous: Option<Timestamp> = None;
    messages
        .iter()
        .map(|message| {
            let gap = previous.map_or(Duration::ZERO, |earlier| {
                Duration::try_from(message.ts.duration_since(earlier)).unwrap_or(Duration::ZERO)
            });
            previous = Some(message.ts);
            let delay = if options.speed > 0.0 && options.speed.is_finite() {
                Duration::try_from_secs_f64(gap.as_secs_f64() / options.speed).unwrap_or(Duration::ZERO)
            } else {
                Duration::ZERO
            };
            let topic = if options.prefix_from.is_empty() {
                message.topic.clone()
            } else {
                topic::rewrite_prefix(&message.topic, &options.prefix_from, &options.prefix_to)
            };
            ReplayStep {
                delay,
                request: PublishRequest {
                    topic,
                    payload: message.payload.clone(),
                    qos: message.qos,
                    retain: message.retain,
                    properties: message.properties.clone().unwrap_or_default(),
                },
            }
        })
        .collect()
}

/// Total playback time of a plan.
#[must_use]
pub fn replay_duration(steps: &[ReplayStep]) -> Duration {
    steps.iter().map(|step| step.delay).sum()
}
