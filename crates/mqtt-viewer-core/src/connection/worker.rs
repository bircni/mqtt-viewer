//! The connection worker: one task that owns all connection state.
//!
//! The network is driven by a separate task per session ([`drive`]) so that a slow connect is never cancelled
//! by the worker reacting to commands or timers.

use std::collections::VecDeque;
use std::sync::mpsc::Sender;
use std::time::Duration;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval, sleep, sleep_until, timeout};

use super::session::{self, Client, EventLoop, Incoming};
use super::{Command, ConnectionState, Event};
use crate::config::{ConnectionProfile, Subscription};
use crate::message::{Message, PublishRequest};
use crate::record::ReplayStep;

/// How often buffered messages are handed to the consumer.
const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
/// A batch this large is flushed without waiting for the timer.
const MAX_BATCH: usize = 1024;
/// How long a graceful disconnect may take before the session task is aborted.
const DISCONNECT_GRACE: Duration = Duration::from_millis(500);
const BACKOFF_START: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// A connection that survives this long resets the back-off.
const STABLE_AFTER: Duration = Duration::from_secs(5);
/// Every this many consecutive short-lived connections the worker reacts: first by giving up `$` subscriptions,
/// then by telling the user.
const FLAPPING_AFTER: u32 = 2;
const FLAPPING_HINT: &str = "The broker keeps closing the connection right after accepting it. It may be rejecting \
                             one of the subscriptions or another client may be using the same client ID.";

/// Delivers events to the consumer and wakes it up.
pub(super) struct EventSink {
    events: Sender<Event>,
    wake: Box<dyn Fn() + Send>,
}

impl EventSink {
    pub(super) fn new(events: Sender<Event>, wake: Box<dyn Fn() + Send>) -> Self {
        Self { events, wake }
    }

    fn send(&self, event: Event) {
        // A closed channel means the owner is being dropped; shutdown follows.
        if self.events.send(event).is_ok() {
            (self.wake)();
        }
    }
}

/// What a session task reports to the worker.
enum SessionEvent {
    Connected,
    Message(Message),
    Warning(String),
    /// The broker accepted the connection and dropped it right away, several times in a row.
    Flapping,
    Retrying {
        attempt: u32,
        error: String,
    },
    Failed(String),
    Closed,
}

/// Delay before reconnect attempt number `attempt` (1-based): doubling from half a second, capped.
pub(super) fn backoff(attempt: u32) -> Duration {
    let factor = 2_u32.saturating_pow(attempt.saturating_sub(1).min(16));
    BACKOFF_START.saturating_mul(factor).min(BACKOFF_MAX)
}

/// Polls one session until it ends, forwarding what happens. `generation` tags the events so the worker can
/// ignore those of a session it already replaced.
async fn drive(mut eventloop: EventLoop, generation: u64, events: UnboundedSender<(u64, SessionEvent)>) {
    let mut attempt = 0_u32;
    let mut connected_at: Option<Instant> = None;
    loop {
        let event = match eventloop.poll().await {
            Ok(Incoming::Connected) => {
                connected_at = Some(Instant::now());
                SessionEvent::Connected
            }
            Ok(Incoming::Publish(message)) => SessionEvent::Message(message),
            Ok(Incoming::Warning(warning)) => SessionEvent::Warning(warning),
            Ok(Incoming::Closed) => {
                let _ = events.send((generation, SessionEvent::Closed));
                return;
            }
            Ok(Incoming::Other) => continue,
            Err(error) if error.fatal => {
                let _ = events.send((generation, SessionEvent::Failed(error.message)));
                return;
            }
            Err(error) => {
                // Only a connection that lasted counts as a success; a broker that accepts and then drops us
                // right away (typically over a subscription it refuses) must not be hammered twice a second.
                let short_lived = connected_at.take().map(|at| at.elapsed() < STABLE_AFTER);
                if short_lived == Some(false) {
                    attempt = 0;
                }
                attempt = attempt.saturating_add(1);
                if short_lived == Some(true) && attempt.is_multiple_of(FLAPPING_AFTER) {
                    let _ = events.send((generation, SessionEvent::Flapping));
                }
                if events.send((generation, SessionEvent::Retrying { attempt, error: error.message })).is_err() {
                    return;
                }
                sleep(backoff(attempt)).await;
                continue;
            }
        };
        if events.send((generation, event)).is_err() {
            return;
        }
    }
}

enum Outgoing {
    Subscribe(Subscription),
    Unsubscribe(String),
    Publish(PublishRequest),
}

struct Active {
    client: Client,
    task: JoinHandle<()>,
    generation: u64,
    connected: bool,
    subscriptions: Vec<Subscription>,
    /// Requests waiting for room in the client's queue.
    outbox: VecDeque<Outgoing>,
}

impl Active {
    /// Sends as much of the outbox as the client accepts right now. While offline everything stays queued, so
    /// that after a reconnect the subscriptions go out before any waiting publish.
    fn pump(&mut self) {
        if !self.connected {
            return;
        }
        while let Some(next) = self.outbox.front() {
            let sent = match next {
                Outgoing::Subscribe(subscription) => self.client.subscribe(&subscription.filter, subscription.qos),
                Outgoing::Unsubscribe(filter) => self.client.unsubscribe(filter),
                Outgoing::Publish(request) => self.client.publish(request),
            };
            if sent.is_err() {
                break;
            }
            self.outbox.pop_front();
        }
    }

    fn close(self) {
        self.client.disconnect();
        let mut task = self.task;
        tokio::spawn(async move {
            if timeout(DISCONNECT_GRACE, &mut task).await.is_err() {
                task.abort();
            }
        });
    }
}

struct Replay {
    steps: VecDeque<ReplayStep>,
    total: usize,
    sent: usize,
    next_at: Instant,
    last_report: Instant,
}

struct Worker {
    sink: EventSink,
    session_tx: UnboundedSender<(u64, SessionEvent)>,
    active: Option<Active>,
    generation: u64,
    batch: Vec<Message>,
    replay: Option<Replay>,
}

impl Worker {
    fn flush(&mut self) {
        if !self.batch.is_empty() {
            self.sink.send(Event::Messages(std::mem::take(&mut self.batch)));
        }
    }

    fn connect(&mut self, profile: &ConnectionProfile) {
        self.disconnect(false);
        match session::open(profile) {
            Ok((client, eventloop)) => {
                self.generation += 1;
                let task = tokio::spawn(drive(eventloop, self.generation, self.session_tx.clone()));
                self.active = Some(Active {
                    client,
                    task,
                    generation: self.generation,
                    connected: false,
                    subscriptions: profile.subscriptions.clone(),
                    outbox: VecDeque::new(),
                });
                self.sink.send(Event::State(ConnectionState::Connecting));
            }
            Err(error) => {
                self.sink.send(Event::Error(error));
                self.sink.send(Event::State(ConnectionState::Disconnected));
            }
        }
    }

    fn disconnect(&mut self, announce: bool) {
        self.stop_replay();
        if let Some(active) = self.active.take() {
            active.close();
            self.flush();
            if announce {
                self.sink.send(Event::State(ConnectionState::Disconnected));
            }
        }
    }

    fn stop_replay(&mut self) {
        if let Some(replay) = self.replay.take() {
            self.sink.send(Event::ReplayFinished { sent: replay.sent, total: replay.total });
        }
    }

    fn handle_command(&mut self, command: Command) {
        match command {
            Command::Connect(profile) => self.connect(&profile),
            Command::Disconnect => self.disconnect(true),
            Command::Subscribe(subscription) => self.with_active("subscribe", |active| {
                active.subscriptions.retain(|existing| existing.filter != subscription.filter);
                active.subscriptions.push(subscription.clone());
                if active.connected {
                    active.outbox.push_back(Outgoing::Subscribe(subscription));
                }
            }),
            Command::Unsubscribe(filter) => self.with_active("unsubscribe", |active| {
                active.subscriptions.retain(|existing| existing.filter != filter);
                if active.connected {
                    active.outbox.push_back(Outgoing::Unsubscribe(filter));
                }
            }),
            Command::Publish(request) => match crate::topic::validate_topic(&request.topic) {
                Ok(()) => self.with_active("publish", |active| active.outbox.push_back(Outgoing::Publish(request))),
                Err(error) => self.sink.send(Event::Error(format!("cannot publish: {error}"))),
            },
            Command::Replay(steps) => {
                self.stop_replay();
                if self.active.is_none() {
                    self.sink.send(Event::Error("cannot replay: not connected".to_owned()));
                    self.sink.send(Event::ReplayFinished { sent: 0, total: steps.len() });
                    return;
                }
                let now = Instant::now();
                let first_delay = steps.first().map_or(Duration::ZERO, |step| step.delay);
                self.replay = Some(Replay {
                    total: steps.len(),
                    steps: steps.into(),
                    sent: 0,
                    next_at: now + first_delay,
                    last_report: now,
                });
            }
            Command::StopReplay => self.stop_replay(),
            // Handled by the caller, which owns the loop.
            Command::Shutdown => {}
        }
    }

    fn with_active(&mut self, action: &str, apply: impl FnOnce(&mut Active)) {
        match &mut self.active {
            Some(active) => {
                apply(active);
                active.pump();
            }
            None => self.sink.send(Event::Error(format!("cannot {action}: not connected"))),
        }
    }

    fn handle_session(&mut self, generation: u64, event: SessionEvent) {
        let Some(active) = self.active.as_mut().filter(|active| active.generation == generation) else {
            return;
        };
        match event {
            SessionEvent::Connected => {
                active.connected = true;
                // Subscriptions go first: with a clean session the broker forgot them.
                for subscription in active.subscriptions.iter().rev() {
                    active.outbox.push_front(Outgoing::Subscribe(subscription.clone()));
                }
                active.pump();
                self.sink.send(Event::State(ConnectionState::Connected));
            }
            SessionEvent::Message(message) => {
                self.batch.push(message);
                if self.batch.len() >= MAX_BATCH {
                    self.flush();
                }
            }
            SessionEvent::Warning(warning) => self.sink.send(Event::Error(warning)),
            SessionEvent::Flapping => {
                // Many brokers (and managed services) hang up on clients that subscribe to `$SYS` or other
                // `$` topics. Those subscriptions are optional extras, so give them up for this session before
                // bothering the user with a problem they would have to diagnose themselves.
                let (system, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut active.subscriptions)
                    .into_iter()
                    .partition(|entry| entry.filter.starts_with('$'));
                active.subscriptions = kept;
                if system.is_empty() {
                    self.sink.send(Event::Error(FLAPPING_HINT.to_owned()));
                } else {
                    let filters: Vec<&str> = system.iter().map(|entry| entry.filter.as_str()).collect();
                    self.sink.send(Event::Error(format!(
                        "The broker closes the connection when this client subscribes to {}. Continuing without \
                         it; remove it from the profile to avoid the delay.",
                        filters.join(", ")
                    )));
                }
            }
            SessionEvent::Retrying { attempt, error } => {
                active.connected = false;
                // Queued subscriptions are re-added on the next connect.
                active.outbox.retain(|outgoing| matches!(outgoing, Outgoing::Publish(_)));
                self.flush();
                self.sink.send(Event::State(ConnectionState::Reconnecting { attempt, error }));
            }
            SessionEvent::Failed(error) => {
                self.active = None;
                self.stop_replay();
                self.flush();
                self.sink.send(Event::Error(error));
                self.sink.send(Event::State(ConnectionState::Disconnected));
            }
            SessionEvent::Closed => {
                self.active = None;
                self.stop_replay();
                self.flush();
                self.sink.send(Event::State(ConnectionState::Disconnected));
            }
        }
    }

    /// Publishes every replay step that is due.
    fn advance_replay(&mut self) {
        let now = Instant::now();
        let Some(replay) = self.replay.as_mut() else { return };
        let Some(active) = self.active.as_mut() else {
            self.stop_replay();
            return;
        };
        while replay.next_at <= now {
            let Some(step) = replay.steps.pop_front() else { break };
            active.outbox.push_back(Outgoing::Publish(step.request));
            replay.sent += 1;
            match replay.steps.front() {
                Some(next) => replay.next_at += next.delay,
                None => break,
            }
        }
        active.pump();
        if replay.steps.is_empty() {
            self.stop_replay();
        } else if now.duration_since(replay.last_report) >= Duration::from_millis(100) {
            replay.last_report = now;
            self.sink.send(Event::ReplayProgress { sent: replay.sent, total: replay.total });
        }
    }
}

/// Runs until [`Command::Shutdown`] arrives or the command channel closes.
pub(super) async fn run(mut commands: UnboundedReceiver<Command>, sink: EventSink) {
    let (session_tx, mut session_rx) = unbounded_channel();
    let mut worker = Worker { sink, session_tx, active: None, generation: 0, batch: Vec::new(), replay: None };
    let mut ticker = interval(FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        let replay_due = worker.replay.as_ref().map(|replay| replay.next_at);
        tokio::select! {
            command = commands.recv() => match command {
                None | Some(Command::Shutdown) => break,
                Some(command) => worker.handle_command(command),
            },
            Some((generation, event)) = session_rx.recv() => worker.handle_session(generation, event),
            _ = ticker.tick() => {
                worker.flush();
                if let Some(active) = worker.active.as_mut() {
                    active.pump();
                }
            }
            () = async { sleep_until(replay_due.unwrap_or_else(Instant::now)).await }, if replay_due.is_some() => {
                worker.advance_replay();
            }
        }
    }

    worker.flush();
    if let Some(active) = worker.active.take() {
        active.client.disconnect();
        // Give the DISCONNECT packet a moment to reach the socket before the runtime goes away.
        let mut task = active.task;
        if timeout(DISCONNECT_GRACE, &mut task).await.is_err() {
            task.abort();
        }
    }
}
