//! Everything the application knows, and every way it can change.
//!
//! The UI never mutates anything but form fields directly; every user intent is an [`Action`] handled by
//! [`AppState::apply`], and everything the broker reports comes in through [`AppState::handle_event`]. Requests
//! to the connection worker are collected in a queue the [`crate::app::App`] drains, which keeps this module
//! free of threads and windows and therefore easy to test.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use mqtt_viewer_core::config::{AppConfig, ConnectionProfile, ProtoBinding, PublishPreset, Settings, TreeOrder};
use mqtt_viewer_core::connection::{Command, ConnectionState, Event};
use mqtt_viewer_core::decode::{Decoded, Decoder, Format, ProtoRegistry};
use mqtt_viewer_core::diff::{self, DiffLine};
use mqtt_viewer_core::message::{Message, PublishRequest};
use mqtt_viewer_core::numeric;
use mqtt_viewer_core::record::{self, RecordedMessage, Recorder, ReplayOptions};
use mqtt_viewer_core::stats::RateMeter;
use mqtt_viewer_core::store::{Retention, Store, StoreWriter};
use mqtt_viewer_core::tree::{Filter, HistoryLimits, NodeId, TopicTree};

use crate::chart::Chart;
use crate::forms::PublishForm;
use crate::picker::FilePicker;

/// Messages buffered while paused before the oldest are dropped.
pub const MAX_HELD: usize = 200_000;
/// Entries kept in the publish history.
pub const MAX_PUBLISH_HISTORY: usize = 20;
/// Decoded text beyond this many bytes is cut off in the value view.
pub const MAX_VALUE_TEXT: usize = 256 * 1024;
/// How long a notice stays on screen.
pub const NOTICE_LIFETIME: Duration = Duration::from_secs(8);
/// Numeric paths offered for charting per payload.
const MAX_CHART_PATHS: usize = 64;

/// A short message for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// The text.
    pub text: String,
    /// Whether it reports a problem.
    pub is_error: bool,
    /// When it was raised.
    pub at: Instant,
}

/// The connection editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectDialog {
    /// The profile being edited.
    pub draft: ConnectionProfile,
    /// Whether the password is shown in clear.
    pub show_password: bool,
}

/// The replay setup shown after a recording was loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayDialog {
    /// File the recording came from.
    pub path: PathBuf,
    /// The recorded messages.
    pub messages: Vec<RecordedMessage>,
    /// Playback options.
    pub options: ReplayOptions,
}

/// The modal dialog currently open.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Dialog {
    /// No dialog.
    #[default]
    None,
    /// Connection profiles.
    Connect(Box<ConnectDialog>),
    /// Application settings, edited as a draft.
    Settings(Box<Settings>),
    /// Confirmation before deleting retained topics.
    ConfirmDelete {
        /// Node the deletion started from.
        node: NodeId,
        /// Whether everything below the node goes as well.
        recursive: bool,
        /// Topics that will be cleared.
        topics: Vec<String>,
    },
    /// Replay setup.
    Replay(Box<ReplayDialog>),
}

/// What the value panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValueMode {
    /// The decoded payload.
    #[default]
    Decoded,
    /// The payload as a hex dump.
    Raw,
    /// Differences to another message.
    Diff,
}

/// Which messages an export covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportScope {
    /// The latest value of the selected topic.
    Value,
    /// The stored history of the selected topic.
    History,
    /// The latest value of every topic at or below the selected one.
    Subtree,
}

/// File format of an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// A JSON array.
    Json,
    /// Comma-separated values.
    Csv,
}

/// Keyboard movement in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Navigate {
    /// Previous row.
    Up,
    /// Next row.
    Down,
    /// Collapse, or go to the parent.
    Left,
    /// Expand, or go to the first child.
    Right,
    /// First row.
    First,
    /// Last row.
    Last,
}

/// A user intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Open the connection dialog, editing the last used profile.
    OpenConnectDialog,
    /// Open the settings dialog.
    OpenSettings,
    /// Close whatever dialog is open, discarding drafts.
    CloseDialog,
    /// Show another saved profile in the connection dialog.
    EditProfile(String),
    /// Start a fresh profile in the connection dialog.
    NewProfile,
    /// Save the connection dialog's draft.
    SaveProfile,
    /// Save a copy of the profile with this identifier and edit it.
    DuplicateProfile(String),
    /// Delete the profile with this identifier.
    DeleteProfile(String),
    /// Save the draft and connect with it.
    Connect,
    /// Disconnect from the broker.
    Disconnect,
    /// Select a topic, or nothing.
    Select(Option<NodeId>),
    /// Expand or collapse a node.
    SetExpanded(NodeId, bool),
    /// Expand or collapse a node and everything below it.
    SetExpandedRecursive(NodeId, bool),
    /// Move the selection with the keyboard.
    Navigate(Navigate),
    /// The search fields changed.
    SearchChanged,
    /// Put the keyboard focus into the search box.
    FocusSearch,
    /// Clear the retained message of the selected topic or, when it has no value of its own, of everything
    /// below it.
    DeleteSelected,
    /// Change the sibling ordering.
    SetOrder(TreeOrder),
    /// Pause or resume applying incoming messages.
    TogglePause,
    /// Remove all topics from the view.
    ClearTree,
    /// Publish what the publish form describes.
    Publish,
    /// Pretty-print the publish form's JSON payload.
    FormatPublishPayload,
    /// Load an entry of the publish history into the form.
    LoadPublishHistory(usize),
    /// Save the publish form as a named preset.
    SavePreset(String),
    /// Load a preset into the form.
    LoadPreset(usize),
    /// Delete a preset.
    DeletePreset(usize),
    /// Copy the selected topic into the publish form.
    UseTopicForPublish(NodeId),
    /// Clear the retained message of a topic, or of everything below it.
    DeleteRetained {
        /// Where to start.
        node: NodeId,
        /// Whether to include everything below.
        recursive: bool,
    },
    /// Go ahead with the deletion the confirmation dialog describes.
    ConfirmDelete,
    /// Switch the value panel's view.
    SetValueMode(ValueMode),
    /// Compare the selected topic's latest value with the history entry having this sequence number;
    /// `None` compares with the previous message.
    CompareWith(Option<u64>),
    /// Force a decoder for a topic filter; [`Format::Auto`] removes the override.
    SetDecoder {
        /// Topic or topic filter.
        pattern: String,
        /// Decoder to use.
        format: Format,
    },
    /// Chart a numeric value.
    AddChart {
        /// Topic to chart.
        topic: String,
        /// JSON Pointer into the payload.
        pointer: String,
    },
    /// Remove a chart.
    RemoveChart(usize),
    /// Start recording received messages to a file.
    StartRecording(PathBuf),
    /// Stop the running recording.
    StopRecording,
    /// Write messages to a file.
    Export {
        /// Which messages.
        scope: ExportScope,
        /// File format.
        format: ExportFormat,
        /// Destination.
        path: PathBuf,
    },
    /// Load a recording and open the replay dialog.
    LoadRecording(PathBuf),
    /// Start the replay the dialog describes.
    StartReplay,
    /// Abort the running replay.
    StopReplay,
    /// Apply and save the settings dialog's draft.
    ApplySettings,
    /// Delete the stored history of all profiles.
    ClearStoredHistory,
    /// Load another `.proto` file.
    AddProtoFile(PathBuf),
    /// Forget a `.proto` file.
    RemoveProtoFile(usize),
    /// Bind a Protobuf message type to a topic filter.
    AddProtoBinding(ProtoBinding),
    /// Remove a Protobuf binding.
    RemoveProtoBinding(usize),
    /// Remove a notice.
    DismissNotice(usize),
}

/// The selected topic's latest value, decoded once and reused across frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueView {
    node: NodeId,
    seq: u64,
    requested: Format,
    /// The decoded payload.
    pub decoded: Decoded,
    /// The text to display, cut off at [`MAX_VALUE_TEXT`].
    pub text: String,
    /// Whether `text` was cut off.
    pub truncated: bool,
    /// JSON Pointers to the numbers in the payload.
    pub chartable: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiffView {
    node: NodeId,
    seq: u64,
    other_seq: u64,
    requested: Format,
    lines: Vec<DiffLine>,
}

/// The whole application state.
#[derive(Debug)]
#[expect(clippy::struct_excessive_bools, reason = "independent flags of unrelated parts of the UI")]
pub struct AppState {
    /// Persistent configuration.
    pub config: AppConfig,
    config_path: Option<PathBuf>,
    database_path: Option<PathBuf>,
    /// All topics of the current connection.
    pub tree: TopicTree,
    /// Payload decoder with the loaded schemas.
    pub decoder: Decoder,
    /// Connection state as last reported.
    pub connection: ConnectionState,
    /// Profile of the current (or last) connection.
    pub profile: Option<ConnectionProfile>,
    /// Selected topic.
    pub selected: Option<NodeId>,
    /// Text of the search box.
    pub search_text: String,
    /// Whether the search also looks at payloads.
    pub search_payload: bool,
    /// Whether incoming messages are held back.
    pub paused: bool,
    held: VecDeque<Message>,
    /// Messages dropped because the pause buffer overflowed.
    pub held_dropped: u64,
    /// The publish panel.
    pub publish: PublishForm,
    /// Recently published requests, newest first.
    pub publish_history: Vec<PublishRequest>,
    /// Open charts.
    pub charts: Vec<Chart>,
    /// Messages for the user, oldest first.
    pub notices: Vec<Notice>,
    /// The open dialog.
    pub dialog: Dialog,
    /// View of the value panel.
    pub value_mode: ValueMode,
    compare_with: Option<u64>,
    value_cache: Option<ValueView>,
    diff_cache: Option<DiffView>,
    recorder: Option<Recorder>,
    /// Progress of the running replay as `(sent, total)`.
    pub replay: Option<(usize, usize)>,
    rate: RateMeter,
    store: Option<StoreWriter>,
    pending_deletes: HashSet<String>,
    /// How the user is asked for files.
    pub files: Box<dyn FilePicker>,
    /// Set when the selection moved by keyboard, so the tree scrolls it into view once.
    pub reveal_selection: bool,
    /// Set when the search box should take the keyboard focus on the next frame.
    pub focus_search: bool,
    commands: Vec<Command>,
}

impl AppState {
    /// A state using `config`, persisted to `config_path` and, when enabled, with history in `database_path`.
    #[must_use]
    pub fn new(config: AppConfig, config_path: Option<PathBuf>, database_path: Option<PathBuf>) -> Self {
        let mut state = Self {
            tree: TopicTree::new(limits(&config.settings)),
            decoder: Decoder::new(),
            config,
            config_path,
            database_path,
            connection: ConnectionState::Disconnected,
            profile: None,
            selected: None,
            search_text: String::new(),
            search_payload: false,
            paused: false,
            held: VecDeque::new(),
            held_dropped: 0,
            publish: PublishForm::default(),
            publish_history: Vec::new(),
            charts: Vec::new(),
            notices: Vec::new(),
            dialog: Dialog::None,
            value_mode: ValueMode::Decoded,
            compare_with: None,
            value_cache: None,
            diff_cache: None,
            recorder: None,
            replay: None,
            rate: RateMeter::default(),
            store: None,
            pending_deletes: HashSet::new(),
            files: Box::new(crate::ui::NativePicker),
            reveal_selection: false,
            focus_search: false,
            commands: Vec::new(),
        };
        state.tree.set_order(state.config.settings.tree_order);
        state.tree.set_auto_expand(state.config.settings.auto_expand_depth);
        state.reload_decoder();
        state.sync_store();
        state
    }

    /// Loads the configuration from its default location and builds the state.
    #[must_use]
    pub fn load() -> Self {
        Self::load_from(AppConfig::default_path(), AppConfig::default_database_path())
    }

    /// Like [`Self::load`], with explicit file locations. A configuration file that cannot be parsed is copied
    /// to `<name>.broken` before the defaults take over, so the next save does not destroy it.
    #[must_use]
    pub fn load_from(path: Option<PathBuf>, database_path: Option<PathBuf>) -> Self {
        let (config, error) = match path.as_deref().map(AppConfig::load) {
            Some(Ok(config)) => (config, None),
            Some(Err(error)) => {
                let mut text = error.to_string();
                if let Some(path) = &path {
                    let mut backup = path.clone().into_os_string();
                    backup.push(".broken");
                    if std::fs::copy(path, &backup).is_ok() {
                        text.push_str(". The file was kept as ");
                        text.push_str(&PathBuf::from(backup).display().to_string());
                    }
                }
                (AppConfig::default(), Some(text))
            }
            None => (AppConfig::default(), None),
        };
        let mut state = Self::new(config, path, database_path);
        if let Some(error) = error {
            state.error(error);
        }
        state
    }

    /// Takes the commands queued for the connection worker.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    /// Raises an informational notice.
    pub fn info(&mut self, text: impl Into<String>) {
        self.notify(text.into(), false);
    }

    /// Raises an error notice.
    pub fn error(&mut self, text: impl Into<String>) {
        self.notify(text.into(), true);
    }

    /// Adds a notice; one that is already showing is refreshed instead of repeated.
    fn notify(&mut self, text: String, is_error: bool) {
        self.notices.retain(|notice| notice.text != text || notice.is_error != is_error);
        self.notices.push(Notice { text, is_error, at: Instant::now() });
    }

    /// Periodic housekeeping: samples the message rate and expires old notices.
    pub fn tick(&mut self, now: Instant) {
        self.rate.sample(now, self.tree.stats().messages);
        self.notices.retain(|notice| now.saturating_duration_since(notice.at) < NOTICE_LIFETIME);
    }

    /// Messages per second over the last few seconds.
    #[must_use]
    pub fn message_rate(&self) -> f64 {
        self.rate.per_second()
    }

    /// Number of messages waiting while paused.
    #[must_use]
    pub fn held_count(&self) -> usize {
        self.held.len()
    }

    /// Whether a recording is running, and how many messages it holds.
    #[must_use]
    pub fn recording(&self) -> Option<(&Path, u64)> {
        self.recorder.as_ref().map(|recorder| (recorder.path(), recorder.count()))
    }

    /// Whether messages are being written to the history database.
    #[must_use]
    pub const fn is_persisting(&self) -> bool {
        self.store.is_some()
    }

    /// Full topic of the selection.
    #[must_use]
    pub fn selected_topic(&self) -> Option<String> {
        self.selected.map(|id| self.tree.topic(id))
    }

    /// The decoder format in force for `topic`.
    #[must_use]
    pub fn format_for(&self, topic: &str) -> Format {
        self.config.decoder_for(topic).unwrap_or(Format::Auto)
    }

    /// The sequence number the diff view compares against, if one was chosen explicitly.
    #[must_use]
    pub const fn compare_target(&self) -> Option<u64> {
        self.compare_with
    }

    fn save_config(&mut self) {
        if let Some(path) = self.config_path.clone()
            && let Err(error) = self.config.save(&path)
        {
            self.error(error.to_string());
        }
    }

    fn reload_decoder(&mut self) {
        let (registry, errors) = ProtoRegistry::load(&self.config.proto_files, &self.config.proto_bindings);
        self.decoder = Decoder::with_registry(registry);
        self.value_cache = None;
        self.diff_cache = None;
        for error in errors {
            self.error(format!("Protobuf schema {error}"));
        }
    }

    /// Opens or closes the history database to match the settings.
    fn sync_store(&mut self) {
        if !self.config.settings.persist_history {
            self.store = None;
            return;
        }
        if self.store.is_some() {
            return;
        }
        let Some(path) = self.database_path.clone() else {
            self.error("History cannot be stored: no data directory is available");
            return;
        };
        match Store::open(&path).map_err(|error| error.to_string()).and_then(|store| {
            StoreWriter::spawn(store).map_err(|error| format!("history writer could not be started: {error}"))
        }) {
            Ok(writer) => {
                writer.prune(retention(&self.config.settings));
                self.store = Some(writer);
            }
            Err(error) => self.error(error),
        }
    }

    /// Handles something the connection worker reported.
    pub fn handle_event(&mut self, event: Event) {
        match event {
            Event::State(state) => {
                match &state {
                    ConnectionState::Connected => {
                        let name = self.profile.as_ref().map_or_else(String::new, ConnectionProfile::url);
                        self.info(format!("Connected to {name}"));
                    }
                    ConnectionState::Reconnecting { attempt, error } => {
                        // One notice per outage is enough; the status bar shows the running attempt count.
                        if *attempt == 1 {
                            self.error(format!("Connection lost: {error}"));
                        }
                    }
                    ConnectionState::Disconnected | ConnectionState::Connecting => {}
                }
                if !state.is_active() {
                    self.replay = None;
                }
                self.connection = state;
            }
            Event::Messages(messages) => self.receive(messages),
            Event::Error(error) => self.error(error),
            Event::ReplayProgress { sent, total } => self.replay = Some((sent, total)),
            Event::ReplayFinished { sent, total } => {
                if self.replay.take().is_some() {
                    self.info(format!("Replay finished: {sent} of {total} messages published"));
                }
            }
        }
    }

    fn receive(&mut self, messages: Vec<Message>) {
        if self.paused {
            self.held.extend(messages);
            while self.held.len() > MAX_HELD {
                self.held.pop_front();
                self.held_dropped += 1;
            }
        } else {
            self.ingest(messages, true);
        }
    }

    /// Applies messages to the tree, the charts and, when `live`, to the recording and the database.
    fn ingest(&mut self, messages: Vec<Message>, live: bool) {
        if live {
            self.record(&messages);
            if let (Some(store), Some(profile)) = (&self.store, &self.profile) {
                store.insert(&profile.id, messages.clone());
            }
        }
        for message in messages {
            if message.payload.is_empty() && self.pending_deletes.remove(&message.topic) {
                // The broker's echo of our own "delete retained" publish; the topic is already gone.
                continue;
            }
            if self.charts.iter().any(|chart| chart.topic == message.topic) {
                let format = self.format_for(&message.topic);
                let decoded = self.decoder.decode(&message.topic, &message.payload, format);
                for chart in self.charts.iter_mut().filter(|chart| chart.topic == message.topic) {
                    chart.feed(message.received_at, &decoded);
                }
            }
            self.tree.insert(message);
        }
    }

    fn record(&mut self, messages: &[Message]) {
        let Some(recorder) = &mut self.recorder else { return };
        let failure = messages.iter().find_map(|message| recorder.write(message).err());
        if let Some(error) = failure {
            self.recorder = None;
            self.error(format!("Recording stopped: {error}"));
        }
    }

    /// The decoded latest value of the selected topic.
    pub fn selected_value(&mut self) -> Option<&ValueView> {
        let node = self.selected.filter(|id| self.tree.contains(*id))?;
        let entry = self.tree.node(node).last()?;
        let topic = self.tree.topic(node);
        let requested = self.format_for(&topic);
        let fresh = self
            .value_cache
            .as_ref()
            .is_some_and(|view| view.node == node && view.seq == entry.seq() && view.requested == requested);
        if !fresh {
            let decoded = self.decoder.decode(&topic, &entry.payload, requested);
            let mut text = decoded.display();
            let truncated = text.len() > MAX_VALUE_TEXT;
            if truncated {
                let cut = (0..=MAX_VALUE_TEXT).rev().find(|index| text.is_char_boundary(*index)).unwrap_or(0);
                text.truncate(cut);
            }
            let chartable = numeric::chartable_paths(&decoded, MAX_CHART_PATHS);
            self.value_cache =
                Some(ValueView { node, seq: entry.seq(), requested, decoded, text, truncated, chartable });
        }
        self.value_cache.as_ref()
    }

    /// The differences between the selected topic's latest value and the comparison target (the chosen history
    /// entry, or the previous message). `None` when there is nothing to compare with.
    pub fn selected_diff(&mut self) -> Option<&[DiffLine]> {
        let node = self.selected.filter(|id| self.tree.contains(*id))?;
        let history = self.tree.node(node).history();
        let latest = history.back()?;
        let other = match self.compare_with {
            Some(seq) => history.iter().find(|entry| entry.seq() == seq)?,
            None => history.iter().rev().nth(1)?,
        };
        let topic = self.tree.topic(node);
        let requested = self.format_for(&topic);
        let fresh = self.diff_cache.as_ref().is_some_and(|view| {
            view.node == node
                && view.seq == latest.seq()
                && view.other_seq == other.seq()
                && view.requested == requested
        });
        if !fresh {
            let old = self.decoder.decode(&topic, &other.payload, requested);
            let new = self.decoder.decode(&topic, &latest.payload, requested);
            self.diff_cache = Some(DiffView {
                node,
                seq: latest.seq(),
                other_seq: other.seq(),
                requested,
                lines: diff::diff_decoded(&old, &new),
            });
        }
        self.diff_cache.as_ref().map(|view| view.lines.as_slice())
    }

    /// Handles a user intent.
    #[expect(clippy::too_many_lines, reason = "one arm per action; splitting it would only scatter the dispatch")]
    pub fn apply(&mut self, action: Action) {
        match action {
            Action::OpenConnectDialog => {
                let draft = self
                    .config
                    .last_profile
                    .as_deref()
                    .and_then(|id| self.config.profile(id))
                    .or_else(|| self.config.profiles.first())
                    .cloned()
                    .unwrap_or_default();
                self.dialog = Dialog::Connect(Box::new(ConnectDialog { draft, show_password: false }));
            }
            Action::OpenSettings => self.dialog = Dialog::Settings(Box::new(self.config.settings.clone())),
            Action::CloseDialog => self.dialog = Dialog::None,
            Action::EditProfile(id) => {
                if let (Dialog::Connect(dialog), Some(profile)) = (&mut self.dialog, self.config.profile(&id)) {
                    dialog.draft = profile.clone();
                }
            }
            Action::NewProfile => {
                self.dialog = Dialog::Connect(Box::new(ConnectDialog {
                    draft: ConnectionProfile::default(),
                    show_password: false,
                }));
            }
            Action::SaveProfile => {
                self.store_draft();
            }
            Action::DuplicateProfile(id) => {
                if let Some(copy) = self.config.profile(&id).map(ConnectionProfile::duplicate) {
                    self.config.profiles.push(copy.clone());
                    self.save_config();
                    self.dialog = Dialog::Connect(Box::new(ConnectDialog { draft: copy, show_password: false }));
                }
            }
            Action::DeleteProfile(id) => {
                self.config.profiles.retain(|profile| profile.id != id);
                if self.config.last_profile.as_deref() == Some(id.as_str()) {
                    self.config.last_profile = None;
                }
                self.save_config();
                if let Dialog::Connect(dialog) = &mut self.dialog
                    && dialog.draft.id == id
                {
                    dialog.draft = self.config.profiles.first().cloned().unwrap_or_default();
                }
            }
            Action::Connect => self.connect(),
            Action::Disconnect => self.commands.push(Command::Disconnect),
            Action::Select(selection) => self.select(selection),
            Action::SetExpanded(node, expanded) => self.tree.set_expanded(node, expanded),
            Action::SetExpandedRecursive(node, expanded) => self.tree.set_expanded_recursive(node, expanded),
            Action::Navigate(direction) => self.navigate(direction),
            Action::SearchChanged => self.tree.set_filter(Filter::new(self.search_text.trim(), self.search_payload)),
            Action::FocusSearch => self.focus_search = true,
            Action::DeleteSelected => {
                // A key is pressed more easily than a button, so this path always asks first.
                if let Some(node) = self.selected.filter(|id| self.tree.contains(*id)) {
                    let recursive = self.tree.node(node).last().is_none();
                    let topics = self.tree.topics_with_values(node, recursive);
                    if !topics.is_empty() {
                        self.dialog = Dialog::ConfirmDelete { node, recursive, topics };
                    }
                }
            }
            Action::SetOrder(order) => {
                self.tree.set_order(order);
                self.config.settings.tree_order = order;
                self.save_config();
            }
            Action::TogglePause => {
                self.paused = !self.paused;
                if !self.paused {
                    let held: Vec<Message> = std::mem::take(&mut self.held).into();
                    if self.held_dropped > 0 {
                        self.error(format!("{} messages were dropped while paused", self.held_dropped));
                        self.held_dropped = 0;
                    }
                    self.ingest(held, true);
                }
            }
            Action::ClearTree => {
                self.tree.clear();
                self.selected = None;
                for chart in &mut self.charts {
                    chart.points.clear();
                }
            }
            Action::Publish => match self.publish.to_request() {
                Ok(request) => {
                    self.publish_history.retain(|earlier| *earlier != request);
                    self.publish_history.insert(0, request.clone());
                    self.publish_history.truncate(MAX_PUBLISH_HISTORY);
                    self.commands.push(Command::Publish(request));
                }
                Err(error) => self.error(error),
            },
            Action::FormatPublishPayload => {
                if let Err(error) = self.publish.format_json() {
                    self.error(error);
                }
            }
            Action::LoadPublishHistory(index) => {
                if let Some(request) = self.publish_history.get(index).cloned() {
                    self.publish.load(&request);
                }
            }
            Action::SavePreset(name) => match self.publish.to_request() {
                Ok(request) if !name.trim().is_empty() => {
                    let name = name.trim().to_owned();
                    self.config.publish_presets.retain(|preset| preset.name != name);
                    self.config.publish_presets.push(PublishPreset { name, request });
                    self.save_config();
                }
                Ok(_) => self.error("A preset needs a name"),
                Err(error) => self.error(error),
            },
            Action::LoadPreset(index) => {
                if let Some(preset) = self.config.publish_presets.get(index).cloned() {
                    self.publish.load(&preset.request);
                }
            }
            Action::DeletePreset(index) => {
                if index < self.config.publish_presets.len() {
                    self.config.publish_presets.remove(index);
                    self.save_config();
                }
            }
            Action::UseTopicForPublish(node) => {
                if self.tree.contains(node) {
                    self.publish.topic = self.tree.topic(node);
                }
            }
            Action::DeleteRetained { node, recursive } => {
                if !self.tree.contains(node) {
                    return;
                }
                let topics = self.tree.topics_with_values(node, recursive);
                if topics.len() > 1 {
                    self.dialog = Dialog::ConfirmDelete { node, recursive, topics };
                } else {
                    self.delete_retained(node, recursive, topics);
                }
            }
            Action::ConfirmDelete => {
                if let Dialog::ConfirmDelete { node, recursive, topics } = std::mem::take(&mut self.dialog) {
                    self.delete_retained(node, recursive, topics);
                }
            }
            Action::SetValueMode(mode) => self.value_mode = mode,
            Action::CompareWith(target) => {
                self.compare_with = target;
                self.value_mode = ValueMode::Diff;
            }
            Action::SetDecoder { pattern, format } => {
                self.config.set_decoder_override(&pattern, (format != Format::Auto).then_some(format));
                self.save_config();
            }
            Action::AddChart { topic, pointer } => self.add_chart(topic, pointer),
            Action::RemoveChart(index) => {
                if index < self.charts.len() {
                    self.charts.remove(index);
                }
            }
            Action::StartRecording(path) => match Recorder::create(&path) {
                Ok(recorder) => {
                    self.recorder = Some(recorder);
                    self.info(format!("Recording to {}", path.display()));
                }
                Err(error) => self.error(format!("Recording could not be started: {error}")),
            },
            Action::StopRecording => {
                if let Some(recorder) = self.recorder.take() {
                    let path = recorder.path().to_owned();
                    match recorder.finish() {
                        Ok(count) => self.info(format!("Recorded {count} messages to {}", path.display())),
                        Err(error) => self.error(format!("Recording is incomplete: {error}")),
                    }
                }
            }
            Action::Export { scope, format, path } => self.export(scope, format, &path),
            Action::LoadRecording(path) => match record::read_recording(&path) {
                Ok(messages) if messages.is_empty() => self.error(format!("{} contains no messages", path.display())),
                Ok(messages) => {
                    self.dialog =
                        Dialog::Replay(Box::new(ReplayDialog { path, messages, options: ReplayOptions::default() }));
                }
                Err(error) => self.error(format!("Recording could not be read: {error}")),
            },
            Action::StartReplay => {
                if let Dialog::Replay(dialog) = std::mem::take(&mut self.dialog) {
                    if self.connection == ConnectionState::Connected {
                        let steps = record::plan_replay(&dialog.messages, &dialog.options);
                        self.replay = Some((0, steps.len()));
                        self.commands.push(Command::Replay(steps));
                    } else {
                        self.error("Connect to a broker before replaying");
                        self.dialog = Dialog::Replay(dialog);
                    }
                }
            }
            Action::StopReplay => self.commands.push(Command::StopReplay),
            Action::ApplySettings => {
                if let Dialog::Settings(draft) = std::mem::take(&mut self.dialog) {
                    self.config.settings = *draft;
                    self.tree.set_limits(limits(&self.config.settings));
                    self.tree.set_order(self.config.settings.tree_order);
                    self.tree.set_auto_expand(self.config.settings.auto_expand_depth);
                    self.sync_store();
                    if let Some(store) = &self.store {
                        store.prune(retention(&self.config.settings));
                    }
                    self.save_config();
                }
            }
            Action::ClearStoredHistory => self.clear_stored_history(),
            Action::AddProtoFile(path) => {
                if !self.config.proto_files.contains(&path) {
                    self.config.proto_files.push(path);
                    self.save_config();
                    self.reload_decoder();
                }
            }
            Action::RemoveProtoFile(index) => {
                if index < self.config.proto_files.len() {
                    self.config.proto_files.remove(index);
                    self.save_config();
                    self.reload_decoder();
                }
            }
            Action::AddProtoBinding(binding) => {
                if let Err(error) = mqtt_viewer_core::topic::validate_filter(&binding.pattern) {
                    self.error(error);
                } else if binding.message_type.trim().is_empty() {
                    self.error("Choose a message type for the binding");
                } else {
                    self.config.proto_bindings.retain(|existing| existing.pattern != binding.pattern);
                    self.config.proto_bindings.push(binding);
                    self.save_config();
                    self.reload_decoder();
                }
            }
            Action::RemoveProtoBinding(index) => {
                if index < self.config.proto_bindings.len() {
                    self.config.proto_bindings.remove(index);
                    self.save_config();
                    self.reload_decoder();
                }
            }
            Action::DismissNotice(index) => {
                if index < self.notices.len() {
                    self.notices.remove(index);
                }
            }
        }
    }

    fn select(&mut self, selection: Option<NodeId>) {
        let previous = self.selected_topic();
        self.selected = selection.filter(|id| self.tree.contains(*id));
        self.compare_with = None;
        // The publish topic follows the selection until the user types a topic of their own.
        let follows = self.publish.topic.is_empty() || Some(&self.publish.topic) == previous.as_ref();
        if let (true, Some(topic)) = (follows, self.selected_topic()) {
            self.publish.topic = topic;
        }
    }

    /// Saves the connection dialog's draft into the profile list. Returns the saved profile when it is valid.
    fn store_draft(&mut self) -> Option<ConnectionProfile> {
        let Dialog::Connect(dialog) = &self.dialog else { return None };
        let draft = dialog.draft.clone();
        if let Err(error) = draft.validate() {
            self.error(format!("Profile is incomplete: {error}"));
            return None;
        }
        match self.config.profiles.iter_mut().find(|profile| profile.id == draft.id) {
            Some(existing) => *existing = draft.clone(),
            None => self.config.profiles.push(draft.clone()),
        }
        self.save_config();
        Some(draft)
    }

    fn connect(&mut self) {
        let Some(profile) = self.store_draft() else { return };
        self.dialog = Dialog::None;
        self.config.last_profile = Some(profile.id.clone());
        self.save_config();

        self.tree.clear();
        self.selected = None;
        self.held.clear();
        self.held_dropped = 0;
        self.pending_deletes.clear();
        for chart in &mut self.charts {
            chart.points.clear();
        }
        self.restore_history(&profile);
        self.profile = Some(profile.clone());
        self.commands.push(Command::Connect(Box::new(profile)));
    }

    /// Fills the tree with what the database holds for `profile`.
    fn restore_history(&mut self, profile: &ConnectionProfile) {
        if self.store.is_none() {
            return;
        }
        let Some(path) = self.database_path.clone() else { return };
        // Make sure everything queued is on disk before reading through a second connection.
        if let Some(store) = &self.store {
            store.flush();
        }
        match Store::open(&path).and_then(|store| store.load_recent(&profile.id, self.config.settings.history_cap)) {
            Ok(messages) => {
                if !messages.is_empty() {
                    self.info(format!("Restored {} stored messages", messages.len()));
                }
                self.ingest(messages, false);
            }
            Err(error) => self.error(error.to_string()),
        }
    }

    fn clear_stored_history(&mut self) {
        if let Some(store) = &self.store {
            store.clear(None);
            self.info("Stored history cleared");
            return;
        }
        // Persistence is off, but an earlier session may have left a database behind.
        let Some(path) = self.database_path.clone().filter(|path| path.exists()) else {
            self.info("There is no stored history");
            return;
        };
        match Store::open(&path).and_then(|mut store| store.clear(None)) {
            Ok(count) => self.info(format!("Stored history cleared ({count} messages)")),
            Err(error) => self.error(error.to_string()),
        }
    }

    fn delete_retained(&mut self, node: NodeId, recursive: bool, topics: Vec<String>) {
        if !self.tree.contains(node) {
            return;
        }
        if self.connection != ConnectionState::Connected {
            self.error("Connect to a broker before deleting retained messages");
            return;
        }
        for topic in topics {
            self.commands.push(Command::Publish(PublishRequest::delete_retained(topic.clone())));
            self.pending_deletes.insert(topic);
        }
        self.tree.remove(node, recursive);
        self.selected = self.selected.filter(|id| self.tree.contains(*id));
    }

    fn add_chart(&mut self, topic: String, pointer: String) {
        if self.charts.iter().any(|chart| chart.topic == topic && chart.pointer == pointer) {
            return;
        }
        let mut chart = Chart::new(topic, pointer);
        if let Some(node) = self.tree.find(&chart.topic) {
            let format = self.format_for(&chart.topic);
            for entry in self.tree.node(node).history() {
                let decoded = self.decoder.decode(&chart.topic, &entry.payload, format);
                chart.feed(entry.received_at, &decoded);
            }
        }
        self.charts.push(chart);
    }

    fn navigate(&mut self, direction: Navigate) {
        let rows = self.tree.rows().to_vec();
        if rows.is_empty() {
            self.selected = None;
            return;
        }
        let current = self.selected.and_then(|id| rows.iter().position(|row| row.id == id));
        let Some(index) = current else {
            self.select(Some(rows[0].id));
            self.reveal_selection = true;
            return;
        };
        let row = rows[index];
        let target = match direction {
            Navigate::Up => index.saturating_sub(1),
            Navigate::Down => (index + 1).min(rows.len() - 1),
            Navigate::First => 0,
            Navigate::Last => rows.len() - 1,
            Navigate::Right => {
                if row.has_children && !row.expanded {
                    self.tree.set_expanded(row.id, true);
                    index
                } else if row.expanded {
                    index + 1
                } else {
                    index
                }
            }
            Navigate::Left => {
                if row.expanded {
                    self.tree.set_expanded(row.id, false);
                    index
                } else {
                    let parent = self.tree.node(row.id).parent();
                    parent.and_then(|parent| rows.iter().position(|candidate| candidate.id == parent)).unwrap_or(index)
                }
            }
        };
        if let Some(target_row) = rows.get(target)
            && target_row.id != row.id
        {
            self.select(Some(target_row.id));
        }
        self.reveal_selection = true;
    }

    fn export(&mut self, scope: ExportScope, format: ExportFormat, path: &Path) {
        let Some(node) = self.selected.filter(|id| self.tree.contains(*id)) else {
            self.error("Select a topic to export");
            return;
        };
        let mut messages: Vec<RecordedMessage> = Vec::new();
        let mut collect = |id: NodeId, latest_only: bool| {
            let topic = self.tree.topic(id);
            let history = self.tree.node(id).history();
            let skip = if latest_only { history.len().saturating_sub(1) } else { 0 };
            messages
                .extend(history.iter().skip(skip).map(|entry| RecordedMessage::from(&entry.to_message(topic.clone()))));
        };
        match scope {
            ExportScope::Value => collect(node, true),
            ExportScope::History => collect(node, false),
            ExportScope::Subtree => {
                for id in self.tree.subtree(node) {
                    collect(id, true);
                }
            }
        }
        if messages.is_empty() {
            self.error("There is nothing to export for this topic");
            return;
        }
        let text = match format {
            ExportFormat::Json => record::export_json(&messages),
            ExportFormat::Csv => record::export_csv(&messages),
        };
        match std::fs::write(path, text) {
            Ok(()) => self.info(format!("Exported {} messages to {}", messages.len(), path.display())),
            Err(error) => self.error(format!("Export to {} failed: {error}", path.display())),
        }
    }

    /// Seconds since a message last arrived at or below `node`, for the activity highlight.
    #[must_use]
    pub fn activity_age(&self, node: NodeId, now: Timestamp) -> Option<f64> {
        self.tree.node(node).last_activity().map(|at| now.duration_since(at).as_secs_f64())
    }
}

const fn limits(settings: &Settings) -> HistoryLimits {
    HistoryLimits {
        per_topic: settings.history_cap,
        total_bytes: settings.memory_budget_mb.saturating_mul(1024 * 1024),
    }
}

fn retention(settings: &Settings) -> Retention {
    Retention {
        max_age_days: settings.retention_days,
        max_bytes: u64::from(settings.retention_mb).saturating_mul(1024 * 1024),
    }
}
