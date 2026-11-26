//! Persistent configuration: connection profiles and application settings.
//!
//! Everything lives in one TOML file. Passwords are stored in clear text by design (the user chose portability
//! over a keychain), so the file is created with owner-only permissions on Unix.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::decode::Format;
use crate::message::{PublishRequest, QoS};

/// Errors from loading or saving the configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read or written.
    #[error("config file {path}: {source}")]
    Io {
        /// File that failed.
        path: PathBuf,
        /// Underlying error.
        source: io::Error,
    },
    /// The file is not valid TOML for this schema.
    #[error("config file {path} is invalid: {source}")]
    Parse {
        /// File that failed.
        path: PathBuf,
        /// Underlying error.
        source: Box<toml::de::Error>,
    },
    /// The configuration could not be encoded.
    #[error("config could not be serialized: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// How the client reaches the broker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// Plain TCP (`mqtt://`).
    #[default]
    Tcp,
    /// TLS over TCP (`mqtts://`).
    Tls,
    /// WebSocket (`ws://`).
    Ws,
    /// WebSocket over TLS (`wss://`).
    Wss,
}

impl Transport {
    /// All transports, in display order.
    pub const ALL: [Self; 4] = [Self::Tcp, Self::Tls, Self::Ws, Self::Wss];

    /// URL scheme shown in the UI.
    #[must_use]
    pub const fn scheme(self) -> &'static str {
        match self {
            Self::Tcp => "mqtt",
            Self::Tls => "mqtts",
            Self::Ws => "ws",
            Self::Wss => "wss",
        }
    }

    /// Conventional port for the transport.
    #[must_use]
    pub const fn default_port(self) -> u16 {
        match self {
            Self::Tcp => 1883,
            Self::Tls => 8883,
            Self::Ws => 80,
            Self::Wss => 443,
        }
    }

    /// Whether the transport is encrypted.
    #[must_use]
    pub const fn is_tls(self) -> bool {
        matches!(self, Self::Tls | Self::Wss)
    }

    /// Whether the transport tunnels through WebSocket.
    #[must_use]
    pub const fn is_websocket(self) -> bool {
        matches!(self, Self::Ws | Self::Wss)
    }
}

/// MQTT protocol revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ProtocolVersion {
    /// MQTT 3.1.1.
    #[default]
    #[serde(rename = "3.1.1")]
    V311,
    /// MQTT 5.0.
    #[serde(rename = "5")]
    V5,
}

impl ProtocolVersion {
    /// Both versions, in display order.
    pub const ALL: [Self; 2] = [Self::V311, Self::V5];

    /// Label shown in the UI.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::V311 => "3.1.1",
            Self::V5 => "5.0",
        }
    }
}

/// One entry of a profile's subscription list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    /// Topic filter.
    pub filter: String,
    /// Maximum QoS requested.
    #[serde(default)]
    pub qos: QoS,
}

impl Subscription {
    /// A subscription to `filter` at QoS 0.
    #[must_use]
    pub fn new(filter: impl Into<String>) -> Self {
        Self { filter: filter.into(), qos: QoS::AtMostOnce }
    }
}

/// Message the broker publishes when the client disappears.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LastWill {
    /// Topic of the will message.
    pub topic: String,
    /// Payload, as text.
    pub payload: String,
    /// QoS of the will message.
    pub qos: QoS,
    /// Whether the will is retained.
    pub retain: bool,
}

/// Everything needed to connect to one broker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionProfile {
    /// Stable identifier, also used as the key for stored history.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Broker host name or address.
    pub host: String,
    /// Broker port.
    pub port: u16,
    /// Transport.
    pub transport: Transport,
    /// Path for WebSocket transports, e.g. `/mqtt`.
    pub ws_path: String,
    /// Protocol revision.
    pub protocol: ProtocolVersion,
    /// User name; empty for anonymous.
    pub username: String,
    /// Password. Stored in clear text.
    pub password: String,
    /// Client identifier; empty means "generate one per connection".
    pub client_id: String,
    /// Keep-alive interval in seconds.
    pub keep_alive_secs: u16,
    /// Clean session (3.1.1) / clean start (5).
    pub clean_session: bool,
    /// Whether the server certificate is verified.
    pub validate_certificate: bool,
    /// PEM file with additional trusted CA certificates.
    pub ca_file: Option<PathBuf>,
    /// PEM file with the client certificate chain.
    pub client_cert_file: Option<PathBuf>,
    /// PEM file with the client private key.
    pub client_key_file: Option<PathBuf>,
    /// Will message, if any.
    pub last_will: Option<LastWill>,
    /// Filters subscribed to after connecting.
    pub subscriptions: Vec<Subscription>,
    /// MQTT 5 session expiry in seconds.
    pub session_expiry_secs: Option<u32>,
    /// MQTT 5 maximum number of topic aliases accepted from the broker.
    pub topic_alias_max: Option<u16>,
}

impl Default for ConnectionProfile {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: "New connection".to_owned(),
            host: "localhost".to_owned(),
            port: Transport::Tcp.default_port(),
            transport: Transport::Tcp,
            ws_path: "/mqtt".to_owned(),
            protocol: ProtocolVersion::V311,
            username: String::new(),
            password: String::new(),
            client_id: String::new(),
            keep_alive_secs: 60,
            clean_session: true,
            validate_certificate: true,
            ca_file: None,
            client_cert_file: None,
            client_key_file: None,
            last_will: None,
            subscriptions: vec![Subscription::new("#"), Subscription::new("$SYS/#")],
            session_expiry_secs: None,
            topic_alias_max: None,
        }
    }
}

impl ConnectionProfile {
    /// The client identifier to connect with: the configured one, or a fresh random one.
    #[must_use]
    pub fn effective_client_id(&self) -> String {
        if self.client_id.trim().is_empty() {
            format!("mqtt-viewer-{}", &new_id()[..8])
        } else {
            self.client_id.clone()
        }
    }

    /// `scheme://host:port` for display.
    #[must_use]
    pub fn url(&self) -> String {
        let path = if self.transport.is_websocket() { self.ws_path.as_str() } else { "" };
        format!("{}://{}:{}{}", self.transport.scheme(), self.host, self.port, path)
    }

    /// A copy with a new identifier and a "(copy)" name.
    #[must_use]
    pub fn duplicate(&self) -> Self {
        Self { id: new_id(), name: format!("{} (copy)", self.name), ..self.clone() }
    }

    /// Checks the profile for problems that would make connecting pointless.
    ///
    /// # Errors
    /// Returns the first problem found, as a message for the user.
    pub fn validate(&self) -> Result<(), String> {
        if self.host.trim().is_empty() {
            return Err("host must not be empty".to_owned());
        }
        if self.port == 0 {
            return Err("port must not be 0".to_owned());
        }
        if self.client_cert_file.is_some() != self.client_key_file.is_some() {
            return Err("client certificate and client key must be set together".to_owned());
        }
        for subscription in &self.subscriptions {
            crate::topic::validate_filter(&subscription.filter)?;
        }
        if let Some(will) = &self.last_will {
            crate::topic::validate_topic(&will.topic)?;
        }
        Ok(())
    }
}

/// A random 32-character hexadecimal identifier.
#[must_use]
pub fn new_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::fmt::Write as _;
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::with_capacity(32);
    for salt in 0..2_u8 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u8(salt);
        hasher.write_i128(jiff::Timestamp::now().as_nanosecond());
        let _ = write!(out, "{:016x}", hasher.finish());
    }
    out
}

/// Colour theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follow the operating system.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

/// How siblings are ordered in the topic tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TreeOrder {
    /// By name.
    #[default]
    Alphabetical,
    /// Most messages first.
    Messages,
    /// Most subtopics first.
    Topics,
}

/// How timestamps are rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimestampFormat {
    /// `14:03:22.123` in local time.
    #[default]
    Time,
    /// `2026-11-25 14:03:22` in local time.
    DateTime,
    /// RFC 3339 in UTC.
    Iso,
    /// `12 s ago`.
    Relative,
}

/// Application-wide settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Colour theme.
    pub theme: Theme,
    /// Sibling ordering in the tree.
    pub tree_order: TreeOrder,
    /// New topics less than this many levels deep start out expanded; 0 leaves the tree collapsed.
    pub auto_expand_depth: usize,
    /// Whether rows flash when a message arrives.
    pub highlight_activity: bool,
    /// Timestamp rendering.
    pub timestamp_format: TimestampFormat,
    /// Maximum number of messages kept per topic.
    pub history_cap: usize,
    /// Upper bound for all kept history, in mebibytes.
    pub memory_budget_mb: usize,
    /// Whether received messages are also written to the local database.
    pub persist_history: bool,
    /// Stored messages older than this many days are pruned; 0 disables the age limit.
    pub retention_days: u32,
    /// The database is pruned to stay under this many mebibytes; 0 disables the size limit.
    pub retention_mb: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            tree_order: TreeOrder::Alphabetical,
            auto_expand_depth: 0,
            highlight_activity: true,
            timestamp_format: TimestampFormat::Time,
            history_cap: 200,
            memory_budget_mb: 256,
            persist_history: false,
            retention_days: 7,
            retention_mb: 512,
        }
    }
}

/// A named, saved publish request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishPreset {
    /// Display name.
    pub name: String,
    /// The request to send.
    pub request: PublishRequest,
}

/// Binds a Protobuf message type to the topics it is published on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtoBinding {
    /// MQTT topic filter (`+` / `#` allowed).
    pub pattern: String,
    /// Fully qualified message name, e.g. `acme.Telemetry`.
    pub message_type: String,
}

/// Forces a decoder for the topics matching a filter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecoderOverride {
    /// MQTT topic filter (`+` / `#` allowed).
    pub pattern: String,
    /// Decoder to use.
    pub format: Format,
}

/// Environment variable naming a directory to keep the configuration and the history database in.
pub const DATA_DIR_VARIABLE: &str = "MQTT_VIEWER_DATA_DIR";

/// The whole configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Application settings.
    pub settings: Settings,
    /// Identifier of the profile used last.
    pub last_profile: Option<String>,
    /// Saved connections.
    pub profiles: Vec<ConnectionProfile>,
    /// Saved publish requests.
    pub publish_presets: Vec<PublishPreset>,
    /// `.proto` files to load at startup.
    pub proto_files: Vec<PathBuf>,
    /// Topic-to-message bindings for Protobuf decoding.
    pub proto_bindings: Vec<ProtoBinding>,
    /// Per-topic decoder overrides.
    pub decoder_overrides: Vec<DecoderOverride>,
}

impl AppConfig {
    /// Default location of the configuration file, if the platform has a config directory.
    ///
    /// Setting the environment variable [`DATA_DIR_VARIABLE`] moves both this file and the history database
    /// into that directory (a "portable" setup).
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        Self::config_path_in(std::env::var_os(DATA_DIR_VARIABLE).map(PathBuf::from))
    }

    /// Default location of the history database; see [`Self::default_path`].
    #[must_use]
    pub fn default_database_path() -> Option<PathBuf> {
        Self::database_path_in(std::env::var_os(DATA_DIR_VARIABLE).map(PathBuf::from))
    }

    /// The configuration file inside `directory`, or in the platform's config directory when `None`.
    #[must_use]
    pub fn config_path_in(directory: Option<PathBuf>) -> Option<PathBuf> {
        directory
            .or_else(|| project_dirs().map(|dirs| dirs.config_dir().to_owned()))
            .map(|base| base.join("config.toml"))
    }

    /// The history database inside `directory`, or in the platform's data directory when `None`.
    #[must_use]
    pub fn database_path_in(directory: Option<PathBuf>) -> Option<PathBuf> {
        directory
            .or_else(|| project_dirs().map(|dirs| dirs.data_dir().to_owned()))
            .map(|base| base.join("history.sqlite"))
    }

    /// Loads the configuration; a missing file yields the defaults.
    ///
    /// # Errors
    /// Fails when the file exists but cannot be read or parsed.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => return Err(ConfigError::Io { path: path.to_owned(), source }),
        };
        toml::from_str(&text).map_err(|source| ConfigError::Parse { path: path.to_owned(), source: Box::new(source) })
    }

    /// Writes the configuration, creating parent directories as needed.
    ///
    /// # Errors
    /// Fails when the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let io_error = |source| ConfigError::Io { path: path.to_owned(), source };
        let text = toml::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        write_private(path, text.as_bytes()).map_err(io_error)
    }

    /// The profile with the given identifier.
    #[must_use]
    pub fn profile(&self, id: &str) -> Option<&ConnectionProfile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    /// The decoder forced for `topic`, if an override matches.
    #[must_use]
    pub fn decoder_for(&self, topic: &str) -> Option<Format> {
        self.decoder_overrides
            .iter()
            .find(|entry| crate::topic::matches(&entry.pattern, topic))
            .map(|entry| entry.format)
    }

    /// Sets or clears (`None`) the decoder override whose pattern is exactly `pattern`.
    pub fn set_decoder_override(&mut self, pattern: &str, format: Option<Format>) {
        self.decoder_overrides.retain(|entry| entry.pattern != pattern);
        if let Some(format) = format {
            self.decoder_overrides.insert(0, DecoderOverride { pattern: pattern.to_owned(), format });
        }
    }
}

fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("", "bircni", "mqtt-viewer")
}

/// Writes `contents` to `path`, readable by the owner only where the platform supports it.
#[cfg(unix)]
fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    // `mode` only applies on creation; tighten a file that already existed.
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(contents)
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    fs::write(path, contents)
}
