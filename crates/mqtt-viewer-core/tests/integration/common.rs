//! Shared helpers for the integration tests: an in-process broker, a killable TCP proxy and event waiting.
#![expect(clippy::unwrap_used, reason = "test helpers should fail loudly")]

use std::fmt::Write as _;
use std::io::{Read as _, Write as _};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mqtt_viewer_core::config::{ConnectionProfile, ProtocolVersion, Subscription, Transport};
use mqtt_viewer_core::connection::{Command, Connection, ConnectionState, Event};
use mqtt_viewer_core::message::{Message, PublishRequest};
use rumqttc::tokio_rustls::TlsAcceptor;
use rumqttc::tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rumqttc::tokio_rustls::rustls::{self, ServerConfig};

/// How long a test waits for something that should happen "immediately".
pub const TIMEOUT: Duration = Duration::from_secs(20);

/// A port nothing is listening on right now.
pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn wait_for_port(port: u16) {
    let deadline = Instant::now() + TIMEOUT;
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "broker did not start listening on port {port}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// What the test broker should offer.
#[derive(Default)]
pub struct BrokerOptions<'a> {
    /// Require these credentials on the 3.1.1 TCP listener.
    pub credentials: Option<(&'a str, &'a str)>,
}

/// An in-process `rumqttd` broker. It cannot be stopped; it lives until the test process exits.
pub struct TestBroker {
    pub v4: u16,
    pub v5: u16,
    pub ws: u16,
}

impl TestBroker {
    pub fn start() -> Self {
        Self::start_with(&BrokerOptions::default())
    }

    pub fn start_with(options: &BrokerOptions<'_>) -> Self {
        let (v4_port, v5_port, ws_port) = (free_port(), free_port(), free_port());
        let connections = "connection_timeout_ms = 60000\nmax_payload_size = 5242880\nmax_inflight_count = 500\ndynamic_filters = true\n";
        let mut config = format!(
            "id = 0\n[router]\nid = 0\nmax_connections = 1000\nmax_outgoing_packet_count = 200\n\
             max_segment_size = 104857600\nmax_segment_count = 10\n\
             [v4.1]\nname = \"v4-1\"\nlisten = \"127.0.0.1:{v4_port}\"\nnext_connection_delay_ms = 1\n\
             [v4.1.connections]\n{connections}"
        );
        if let Some((user, password)) = options.credentials {
            let _ = writeln!(config, "[v4.1.connections.auth]\n{user} = \"{password}\"");
        }
        let _ = write!(
            config,
            "[v5.1]\nname = \"v5-1\"\nlisten = \"127.0.0.1:{v5_port}\"\nnext_connection_delay_ms = 1\n\
             [v5.1.connections]\n{connections}\
             [ws.1]\nname = \"ws-1\"\nlisten = \"127.0.0.1:{ws_port}\"\nnext_connection_delay_ms = 1\n\
             [ws.1.connections]\n{connections}"
        );
        let config: rumqttd::Config = toml::from_str(&config).unwrap();
        std::thread::spawn(move || {
            let mut broker = rumqttd::Broker::new(config);
            broker.start().unwrap();
        });
        for port in [v4_port, v5_port, ws_port] {
            wait_for_port(port);
        }
        Self { v4: v4_port, v5: v5_port, ws: ws_port }
    }

    /// A profile for the plain 3.1.1 listener, subscribed to everything under `test/`.
    pub fn profile(&self) -> ConnectionProfile {
        profile_for(self.v4)
    }

    pub fn v5_profile(&self) -> ConnectionProfile {
        ConnectionProfile { protocol: ProtocolVersion::V5, ..profile_for(self.v5) }
    }

    pub fn ws_profile(&self) -> ConnectionProfile {
        ConnectionProfile { transport: Transport::Ws, ws_path: "/mqtt".into(), ..profile_for(self.ws) }
    }
}

pub fn profile_for(port: u16) -> ConnectionProfile {
    ConnectionProfile {
        host: "127.0.0.1".into(),
        port,
        subscriptions: vec![Subscription::new("test/#")],
        keep_alive_secs: 5,
        ..ConnectionProfile::default()
    }
}

/// A fake broker that answers every connection attempt with the given bytes (a CONNACK refusal) and hangs up.
pub fn refusing_broker(response: &'static [u8]) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut connect = [0_u8; 1024];
            let _ = stream.read(&mut connect);
            let _ = stream.write_all(response);
            let _ = stream.shutdown(Shutdown::Write);
        }
    });
    port
}

/// Forwards TCP connections to a target port and can be cut and restored, to simulate a broker outage.
pub struct TcpProxy {
    pub port: u16,
    target: u16,
    state: Arc<ProxyState>,
}

struct ProxyState {
    down: AtomicBool,
    streams: Mutex<Vec<TcpStream>>,
    /// Every byte the clients sent towards the broker.
    sent: Mutex<Vec<u8>>,
}

impl TcpProxy {
    pub fn start(target: u16) -> Self {
        let port = free_port();
        let state = Arc::new(ProxyState {
            down: AtomicBool::new(false),
            streams: Mutex::new(Vec::new()),
            sent: Mutex::new(Vec::new()),
        });
        let proxy = Self { port, target, state };
        proxy.listen();
        proxy
    }

    fn listen(&self) {
        let address = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = TcpListener::bind(address).unwrap();
        listener.set_nonblocking(true).unwrap();
        let (state, target) = (Arc::clone(&self.state), self.target);
        std::thread::spawn(move || {
            while !state.down.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((client, _)) => {
                        client.set_nonblocking(false).unwrap();
                        let Ok(server) = TcpStream::connect(("127.0.0.1", target)) else { continue };
                        let mut streams = state.streams.lock().unwrap();
                        streams.push(client.try_clone().unwrap());
                        streams.push(server.try_clone().unwrap());
                        drop(streams);
                        pipe(client.try_clone().unwrap(), server.try_clone().unwrap(), Some(Arc::clone(&state)));
                        pipe(server, client, None);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });
    }

    /// Drops all connections and stops accepting new ones.
    pub fn cut(&self) {
        self.state.down.store(true, Ordering::SeqCst);
        for stream in self.state.streams.lock().unwrap().drain(..) {
            let _ = stream.shutdown(Shutdown::Both);
        }
        // Let the accept loop notice and release the port.
        std::thread::sleep(Duration::from_millis(100));
    }

    /// The PUBLISH packets the clients sent so far, as `(topic, qos, retain)` read from the wire.
    /// Only meaningful for MQTT 3.1.1 traffic of a single client.
    pub fn published(&self) -> Vec<(String, u8, bool)> {
        let bytes = self.state.sent.lock().unwrap().clone();
        let mut packets = Vec::new();
        let mut position = 0;
        while position < bytes.len() {
            let header = bytes[position];
            // Remaining length: up to four bytes, seven bits each, high bit means "more".
            let (mut length, mut shift, mut cursor) = (0_usize, 0, position + 1);
            while let Some(byte) = bytes.get(cursor) {
                length |= usize::from(byte & 0x7f) << shift;
                shift += 7;
                cursor += 1;
                if byte & 0x80 == 0 {
                    break;
                }
            }
            let Some(body) = bytes.get(cursor..cursor + length) else { break };
            if header >> 4 == 3 {
                let topic_length = usize::from(u16::from_be_bytes([body[0], body[1]]));
                let topic = String::from_utf8_lossy(&body[2..2 + topic_length]).into_owned();
                packets.push((topic, (header >> 1) & 0b11, header & 1 == 1));
            }
            position = cursor + length;
        }
        packets
    }

    /// The flags byte of the first CONNECT packet a client sent: 0x80 marks a user name, 0x40 a password.
    pub fn connect_flags(&self) -> Option<u8> {
        let bytes = self.state.sent.lock().unwrap().clone();
        // Fixed header 0x10, a remaining length of at most two bytes here, then the protocol name "MQTT"
        // (2 + 4 bytes), the protocol level and the flags.
        let length_bytes = if bytes.get(1)? & 0x80 == 0 { 1 } else { 2 };
        (bytes.first()? >> 4 == 1).then(|| bytes.get(1 + length_bytes + 7).copied()).flatten()
    }

    /// Accepts connections again.
    pub fn restore(&self) {
        self.state.down.store(false, Ordering::SeqCst);
        self.listen();
    }
}

fn pipe(mut from: TcpStream, mut to: TcpStream, capture: Option<Arc<ProxyState>>) {
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match from.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if let Some(state) = &capture {
                        state.sent.lock().unwrap().extend_from_slice(&buffer[..read]);
                    }
                    if to.write_all(&buffer[..read]).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = to.shutdown(Shutdown::Both);
    });
}

/// A connection plus everything it reported so far.
pub struct Client {
    pub connection: Connection,
    pub log: Vec<Event>,
}

impl Client {
    pub fn new() -> Self {
        Self { connection: Connection::spawn(|| {}).unwrap(), log: Vec::new() }
    }

    /// A client that is connected with `profile`.
    pub fn connected(profile: &ConnectionProfile) -> Self {
        let mut client = Self::new();
        client.send(Command::Connect(Box::new(profile.clone())));
        client.wait_state(&ConnectionState::Connected);
        client
    }

    /// A client that is connected with `profile` and that the broker delivers to already. `Connected` is
    /// reported on the broker's greeting, before the subscriptions are in place there, so a message that
    /// another client publishes right afterwards can be lost. A message of its own cannot: the broker handles
    /// it after the subscriptions, so once it is back, they are active. `profile` must subscribe to `test/#`.
    pub fn listening(profile: &ConnectionProfile) -> Self {
        static PROBES: AtomicUsize = AtomicUsize::new(0);
        let mut client = Self::connected(profile);
        let topic = format!("test/listening/{}", PROBES.fetch_add(1, Ordering::Relaxed));
        client.send(Command::Publish(PublishRequest { topic: topic.clone(), ..Default::default() }));
        client.wait_message(&topic);
        client
    }

    pub fn send(&self, command: Command) {
        self.connection.send(command);
    }

    /// Waits until an event satisfies `accept`, returning what it extracted. Panics with the event log on timeout.
    pub fn wait<T>(&mut self, what: &str, mut accept: impl FnMut(&Event) -> Option<T>) -> T {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let Some(event) = self.connection.wait_event(remaining) else {
                panic!("timed out waiting for {what}; events so far: {:#?}", self.log);
            };
            self.log.push(event);
            if let Some(found) = self.log.last().and_then(&mut accept) {
                return found;
            }
        }
    }

    pub fn wait_state(&mut self, expected: &ConnectionState) {
        self.wait(&format!("state {expected:?}"), |event| (*event == Event::State(expected.clone())).then_some(()));
    }

    /// Waits for a message on `topic`.
    pub fn wait_message(&mut self, topic: &str) -> Message {
        self.wait_messages(topic, 1).remove(0)
    }

    /// Waits until `count` messages on `topic` have arrived since the client was created and returns all of
    /// them in order. Earlier events count too, since one batch often carries messages for several topics.
    pub fn wait_messages(&mut self, topic: &str, count: usize) -> Vec<Message> {
        let on_topic = |log: &[Event]| -> Vec<Message> {
            log.iter()
                .filter_map(|event| if let Event::Messages(batch) = event { Some(batch) } else { None })
                .flatten()
                .filter(|message| message.topic == topic)
                .cloned()
                .collect()
        };
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let found = on_topic(&self.log);
            if found.len() >= count {
                return found;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let Some(event) = self.connection.wait_event(remaining) else {
                panic!("timed out waiting for {count} message(s) on {topic}; events so far: {:#?}", self.log);
            };
            self.log.push(event);
        }
    }

    pub fn wait_error(&mut self) -> String {
        self.wait("an error", |event| if let Event::Error(error) = event { Some(error.clone()) } else { None })
    }

    /// Collects events for `duration` and returns the messages among them.
    pub fn messages_within(&mut self, duration: Duration) -> Vec<Message> {
        let deadline = Instant::now() + duration;
        let mut messages = Vec::new();
        while let Some(event) = self.connection.wait_event(deadline.saturating_duration_since(Instant::now())) {
            if let Event::Messages(batch) = &event {
                messages.extend(batch.iter().cloned());
            }
            self.log.push(event);
        }
        messages
    }
}

/// A throw-away certificate authority with a server and a client certificate, written as PEM files.
pub struct TestPki {
    pub directory: tempfile::TempDir,
    server_chain: Vec<CertificateDer<'static>>,
    server_key: Vec<u8>,
    ca: CertificateDer<'static>,
}

impl TestPki {
    pub fn generate() -> Self {
        Self::generate_for(&["localhost", "127.0.0.1"])
    }

    /// Like [`Self::generate`], with a server certificate valid for exactly `names`.
    pub fn generate_for(names: &[&str]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let mut ca_params = rcgen::CertificateParams::new(Vec::new()).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca_params.distinguished_name.push(rcgen::DnType::CommonName, "mqtt-viewer test CA");
        ca_params.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign, rcgen::KeyUsagePurpose::DigitalSignature];
        let ca_certificate = ca_params.self_signed(&ca_key).unwrap();
        let issuer = rcgen::Issuer::new(ca_params, ca_key);

        let server_key = rcgen::KeyPair::generate().unwrap();
        let server_params =
            rcgen::CertificateParams::new(names.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>()).unwrap();
        let server_certificate = server_params.signed_by(&server_key, &issuer).unwrap();

        let client_key = rcgen::KeyPair::generate().unwrap();
        let mut client_params = rcgen::CertificateParams::new(Vec::new()).unwrap();
        client_params.distinguished_name.push(rcgen::DnType::CommonName, "test client");
        client_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
        let client_certificate = client_params.signed_by(&client_key, &issuer).unwrap();

        std::fs::write(directory.path().join("ca.pem"), ca_certificate.pem()).unwrap();
        std::fs::write(directory.path().join("client.pem"), client_certificate.pem()).unwrap();
        std::fs::write(directory.path().join("client.key"), client_key.serialize_pem()).unwrap();
        Self {
            directory,
            server_chain: vec![server_certificate.der().clone()],
            server_key: server_key.serialize_der(),
            ca: ca_certificate.der().clone(),
        }
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }

    fn server_config(&self, require_client_certificate: bool) -> ServerConfig {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let builder =
            ServerConfig::builder_with_provider(Arc::clone(&provider)).with_safe_default_protocol_versions().unwrap();
        let builder = if require_client_certificate {
            let mut roots = rustls::RootCertStore::empty();
            roots.add(self.ca.clone()).unwrap();
            let verifier =
                rustls::server::WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider).build().unwrap();
            builder.with_client_cert_verifier(verifier)
        } else {
            builder.with_no_client_auth()
        };
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.server_key.clone()));
        builder.with_single_cert(self.server_chain.clone(), key).unwrap()
    }
}

/// Terminates TLS and forwards the plain stream to `target`, so any listener of the test broker can be reached
/// over TLS with full control over the server's certificate requirements.
pub fn tls_proxy(pki: &TestPki, target: u16, require_client_certificate: bool) -> u16 {
    let acceptor = TlsAcceptor::from(Arc::new(pki.server_config(require_client_certificate)));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            while let Ok((client, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    // A failed handshake is exactly what some tests provoke.
                    let Ok(mut secure) = acceptor.accept(client).await else { return };
                    let Ok(mut upstream) = tokio::net::TcpStream::connect(("127.0.0.1", target)).await else { return };
                    let _ = tokio::io::copy_bidirectional(&mut secure, &mut upstream).await;
                });
            }
        });
    });
    port
}
