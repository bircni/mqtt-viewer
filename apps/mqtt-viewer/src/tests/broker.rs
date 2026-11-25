//! An in-process broker for the end-to-end tests.

use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// Starts a broker on a free port and returns the port. The broker lives until the test process exits.
pub fn start() -> u16 {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let config = format!(
        "id = 0\n[router]\nid = 0\nmax_connections = 100\nmax_outgoing_packet_count = 200\n\
         max_segment_size = 104857600\nmax_segment_count = 10\n\
         [v4.1]\nname = \"v4-1\"\nlisten = \"127.0.0.1:{port}\"\nnext_connection_delay_ms = 1\n\
         [v4.1.connections]\nconnection_timeout_ms = 60000\nmax_payload_size = 1048576\n\
         max_inflight_count = 500\ndynamic_filters = true\n"
    );
    let config: rumqttd::Config = toml::from_str(&config).unwrap();
    std::thread::spawn(move || rumqttd::Broker::new(config).start().unwrap());
    let deadline = Instant::now() + Duration::from_secs(20);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "broker did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    port
}
