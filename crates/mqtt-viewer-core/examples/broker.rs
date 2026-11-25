//! A local MQTT broker for trying the app without installing one.
//!
//! ```text
//! cargo run -p mqtt-viewer-core --example broker [port]
//! ```
//!
//! Listens for MQTT 3.1.1 on `port` (default 1883), MQTT 5 on `port + 1` and WebSocket on `port + 2`.
//! This is the same in-process broker the tests use; it is not meant for production.

use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let port: u16 = std::env::args().nth(1).map_or(Ok(1883), |argument| argument.parse())?;
    let listener = |section: &str, name: &str, port: u16| {
        format!(
            "[{section}]\nname = \"{name}\"\nlisten = \"127.0.0.1:{port}\"\nnext_connection_delay_ms = 1\n\
             [{section}.connections]\nconnection_timeout_ms = 60000\nmax_payload_size = 5242880\n\
             max_inflight_count = 500\ndynamic_filters = true\n"
        )
    };
    let config = format!(
        "id = 0\n[router]\nid = 0\nmax_connections = 1000\nmax_outgoing_packet_count = 200\n\
         max_segment_size = 104857600\nmax_segment_count = 10\n{}{}{}",
        listener("v4.1", "mqtt", port),
        listener("v5.1", "mqtt5", port + 1),
        listener("ws.1", "websocket", port + 2),
    );
    println!("MQTT 3.1.1 on 127.0.0.1:{port}, MQTT 5 on :{}, WebSocket on :{}", port + 1, port + 2);
    println!(
        "Note: this broker drops clients that subscribe to `$SYS/#`; the app carries on without that subscription."
    );
    rumqttd::Broker::new(toml::from_str(&config)?).start()?;
    Ok(())
}
