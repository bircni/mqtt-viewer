//! The broker connection.
//!
//! [`Connection::spawn`] starts a background thread with its own async runtime. The owner talks to it through
//! [`Command`]s and reads [`Event`]s back; received messages arrive in batches so a busy broker does not flood
//! the consumer with wake-ups. Protocol version and transport are hidden behind the [`session`] module.

mod session;
mod tls;
mod worker;

use std::io;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;

use crate::config::{ConnectionProfile, Subscription};
use crate::message::{Message, PublishRequest};
use crate::record::ReplayStep;

/// A request to the connection worker.
#[derive(Debug, Clone)]
pub enum Command {
    /// Connect with this profile, dropping any current connection first.
    Connect(Box<ConnectionProfile>),
    /// Disconnect and stop reconnecting.
    Disconnect,
    /// Subscribe to a filter; it is re-subscribed after reconnects.
    Subscribe(Subscription),
    /// Unsubscribe from a filter.
    Unsubscribe(String),
    /// Publish a message.
    Publish(PublishRequest),
    /// Publish these steps with their delays; replaces a running replay.
    Replay(Vec<ReplayStep>),
    /// Abort the running replay.
    StopReplay,
    /// Stop the worker thread.
    Shutdown,
}

/// Where the connection currently stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConnectionState {
    /// Not connected and not trying to.
    #[default]
    Disconnected,
    /// First connection attempt in progress.
    Connecting,
    /// Connected and subscribed.
    Connected,
    /// The connection failed or was lost; another attempt follows after a back-off.
    Reconnecting {
        /// Number of consecutive failed attempts.
        attempt: u32,
        /// Why the last attempt failed.
        error: String,
    },
}

impl ConnectionState {
    /// Whether a connection exists or is being (re-)established.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        !matches!(self, Self::Disconnected)
    }
}

/// Something the worker reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The connection state changed.
    State(ConnectionState),
    /// Messages received since the last batch, in arrival order.
    Messages(Vec<Message>),
    /// Something went wrong; the text is meant for the user.
    Error(String),
    /// Progress of a running replay.
    ReplayProgress {
        /// Steps published so far.
        sent: usize,
        /// Steps in the plan.
        total: usize,
    },
    /// The replay ended, either completely or because it was stopped.
    ReplayFinished {
        /// Steps published.
        sent: usize,
        /// Steps in the plan.
        total: usize,
    },
}

/// Handle to the connection worker thread. Dropping it shuts the worker down.
#[derive(Debug)]
pub struct Connection {
    commands: UnboundedSender<Command>,
    events: Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}

impl Connection {
    /// Starts the worker. `wake` is called (from the worker thread) whenever new events are available.
    ///
    /// # Errors
    /// Fails when the thread or its runtime cannot be created.
    pub fn spawn(wake: impl Fn() + Send + 'static) -> io::Result<Self> {
        let (commands, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, events) = std::sync::mpsc::channel();
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        let thread = std::thread::Builder::new().name("mqtt-connection".to_owned()).spawn(move || {
            runtime.block_on(worker::run(command_rx, worker::EventSink::new(event_tx, Box::new(wake))));
        })?;
        Ok(Self { commands, events, thread: Some(thread) })
    }

    /// Sends a command. Commands sent after the worker stopped are dropped.
    pub fn send(&self, command: Command) {
        if self.commands.send(command).is_err() {
            log::warn!("connection worker is gone; command dropped");
        }
    }

    /// The next pending event, without waiting.
    #[must_use]
    pub fn try_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// The next event, waiting up to `timeout` for one.
    #[must_use]
    pub fn wait_event(&self, timeout: Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            log::error!("connection worker panicked");
        }
    }
}
