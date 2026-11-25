use std::path::PathBuf;

use crate::config::{
    AppConfig, ConfigError, ConnectionProfile, LastWill, ProtocolVersion, PublishPreset, Subscription, Theme,
    Transport, new_id,
};
use crate::decode::Format;
use crate::message::{PublishRequest, QoS};

#[test]
fn transports_have_schemes_and_ports() {
    let table: Vec<_> =
        Transport::ALL.iter().map(|t| (t.scheme(), t.default_port(), t.is_tls(), t.is_websocket())).collect();
    assert_eq!(
        table,
        [("mqtt", 1883, false, false), ("mqtts", 8883, true, false), ("ws", 80, false, true), ("wss", 443, true, true)]
    );
    assert_eq!(ProtocolVersion::ALL.map(ProtocolVersion::label), ["3.1.1", "5.0"]);
}

#[test]
fn default_profile_subscribes_to_everything() {
    let profile = ConnectionProfile::default();
    let filters: Vec<&str> = profile.subscriptions.iter().map(|s| s.filter.as_str()).collect();
    assert_eq!(filters, ["#", "$SYS/#"]);
    assert_eq!(profile.url(), "mqtt://localhost:1883");
    assert_eq!(profile.validate(), Ok(()));
    assert_eq!(profile.id.len(), 32);
}

#[test]
fn websocket_url_includes_the_path() {
    let profile =
        ConnectionProfile { transport: Transport::Wss, port: 443, host: "broker".into(), ..Default::default() };
    assert_eq!(profile.url(), "wss://broker:443/mqtt");
}

#[test]
fn client_id_is_generated_when_blank() {
    let blank = ConnectionProfile { client_id: "  ".into(), ..Default::default() };
    let first = blank.effective_client_id();
    assert!(first.starts_with("mqtt-viewer-"), "{first}");
    assert_ne!(first, blank.effective_client_id());
    let fixed = ConnectionProfile { client_id: "me".into(), ..Default::default() };
    assert_eq!(fixed.effective_client_id(), "me");
}

#[test]
fn duplicate_gets_new_identity() {
    let profile = ConnectionProfile { name: "Home".into(), ..Default::default() };
    let copy = profile.duplicate();
    assert_ne!(copy.id, profile.id);
    assert_eq!(copy.name, "Home (copy)");
    assert_eq!(copy.host, profile.host);
}

#[test]
fn validation_reports_the_first_problem() {
    let base = ConnectionProfile::default;
    assert!(ConnectionProfile { host: " ".into(), ..base() }.validate().unwrap_err().contains("host"));
    assert!(ConnectionProfile { port: 0, ..base() }.validate().unwrap_err().contains("port"));
    let half_auth = ConnectionProfile { client_cert_file: Some(PathBuf::from("c.pem")), ..base() };
    assert!(half_auth.validate().unwrap_err().contains("together"));
    let bad_filter = ConnectionProfile { subscriptions: vec![Subscription::new("a/#/b")], ..base() };
    assert!(bad_filter.validate().is_err());
    let bad_will =
        ConnectionProfile { last_will: Some(LastWill { topic: "a/#".into(), ..Default::default() }), ..base() };
    assert!(bad_will.validate().is_err());
}

#[test]
fn ids_are_unique_hex() {
    let (a, b) = (new_id(), new_id());
    assert_ne!(a, b);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()), "{a}");
}

#[test]
fn missing_file_yields_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let config = AppConfig::load(&dir.path().join("nope.toml")).unwrap();
    assert_eq!(config, AppConfig::default());
    assert_eq!(config.settings.theme, Theme::System);
}

#[test]
fn config_round_trips_through_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("config.toml");
    let mut config = AppConfig::default();
    let profile = ConnectionProfile {
        name: "Prod".into(),
        password: "s3cret".into(),
        transport: Transport::Tls,
        protocol: ProtocolVersion::V5,
        ca_file: Some(PathBuf::from("/tmp/ca.pem")),
        last_will: Some(LastWill { topic: "bye".into(), payload: "gone".into(), qos: QoS::AtLeastOnce, retain: true }),
        session_expiry_secs: Some(60),
        ..Default::default()
    };
    config.last_profile = Some(profile.id.clone());
    config.profiles.push(profile.clone());
    config.publish_presets.push(PublishPreset { name: "on".into(), request: PublishRequest::delete_retained("x") });
    config.proto_files.push(PathBuf::from("a.proto"));
    config.set_decoder_override("bin/#", Some(Format::Hex));
    config.save(&path).unwrap();

    let loaded = AppConfig::load(&path).unwrap();
    assert_eq!(loaded, config);
    assert_eq!(loaded.profile(&profile.id), Some(&profile));
    assert_eq!(loaded.profile("missing"), None);

    // Saving again over the existing file works too.
    config.save(&path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "config holds passwords and must be private");
    }
}

#[cfg(unix)]
#[test]
fn saving_tightens_an_existing_world_readable_file() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    AppConfig::default().save(&path).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn broken_files_are_reported_with_their_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "profiles = 3").unwrap();
    let error = AppConfig::load(&path).unwrap_err();
    assert!(matches!(error, ConfigError::Parse { .. }), "{error}");
    assert!(error.to_string().contains("config.toml"), "{error}");

    let error = AppConfig::load(dir.path()).unwrap_err();
    assert!(matches!(error, ConfigError::Io { .. }), "{error}");
    let error = AppConfig::default().save(dir.path()).unwrap_err();
    assert!(matches!(error, ConfigError::Io { .. }), "{error}");
}

#[test]
fn partial_files_fill_in_defaults() {
    let config: AppConfig = toml::from_str("[[profiles]]\nname = \"x\"\nhost = \"h\"\n").unwrap();
    assert_eq!(config.profiles[0].port, 1883);
    assert_eq!(config.profiles[0].keep_alive_secs, 60);
    assert!(config.profiles[0].validate_certificate);
}

#[test]
fn decoder_overrides_match_by_filter_and_can_be_cleared() {
    let mut config = AppConfig::default();
    config.set_decoder_override("sensors/+/raw", Some(Format::Hex));
    config.set_decoder_override("sensors/#", Some(Format::Cbor));
    // The most recently set override wins.
    assert_eq!(config.decoder_for("sensors/a/raw"), Some(Format::Cbor));
    config.set_decoder_override("sensors/#", None);
    assert_eq!(config.decoder_for("sensors/a/raw"), Some(Format::Hex));
    assert_eq!(config.decoder_for("other"), None);
    config.set_decoder_override("sensors/+/raw", Some(Format::Json));
    assert_eq!(config.decoder_overrides.len(), 1);
}

#[test]
fn default_paths_end_in_expected_names() {
    if let Some(path) = AppConfig::default_path() {
        assert!(path.ends_with("config.toml"), "{}", path.display());
    }
    if let Some(path) = AppConfig::default_database_path() {
        assert!(path.ends_with("history.sqlite"), "{}", path.display());
    }
}

#[test]
fn a_data_directory_override_holds_both_files() {
    let portable = PathBuf::from("portable");
    assert_eq!(AppConfig::config_path_in(Some(portable.clone())), Some(portable.join("config.toml")));
    assert_eq!(AppConfig::database_path_in(Some(portable.clone())), Some(portable.join("history.sqlite")));
    if let (Some(config), Some(database)) = (AppConfig::config_path_in(None), AppConfig::database_path_in(None)) {
        assert!(!config.starts_with(&portable) && !database.starts_with(&portable));
    }
}
