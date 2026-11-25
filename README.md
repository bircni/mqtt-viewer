# mqtt-viewer

A native MQTT client for macOS, Windows and Linux that shows everything on a broker as a live topic tree.
It speaks MQTT 3.1.1 and 5, decodes binary payloads and records and replays sessions. Written in Rust with
[egui](https://github.com/emilk/egui); no web view, no Electron.

## Features

- **Connections**: saved profiles; `mqtt://`, `mqtts://`, `ws://` and `wss://`; MQTT 3.1.1 and 5; user name and
  password, client ID, keep-alive, clean session, last will, a subscription list with QoS per entry; custom CA,
  client certificate and key, optional certificate validation; automatic reconnect with back-off. Brokers that
  hang up on `$SYS` subscriptions are handled by dropping those for the session.
- **Topic tree**: collapsible hierarchy with message and subtopic counts, an inline value preview and a flash when
  a message arrives. Ordering by name, message count or topic count. Keyboard navigation, pause/resume, and a
  context menu to copy, publish to, expand or delete. Only visible rows are laid out, so 50 000 topics at
  5 000 messages per second stay smooth.
- **Search**: filter by topic and, optionally, by the latest payload.
- **Value view**: decoded payload with JSON colouring, raw hex dump, QoS, retain flag, timestamp and size, and a
  diff against the previous message or any message from the history.
- **Decoders**: text, JSON, hex, Base64, MessagePack, CBOR, Protobuf (load your own `.proto` files and bind
  message types to topic filters) and Sparkplug B. Detected automatically, overridable per topic.
- **Publish**: raw, JSON, XML or hex payloads with validation, QoS and retain, a history of what you sent and
  named presets. On MQTT 5 also user properties, content type, response topic, correlation data and expiry.
- **Charts**: plot any numeric topic or any number inside a JSON payload, with a time window, a fixed value range,
  stepped or linear lines and pause.
- **Record, export, replay**: record a session to a JSON Lines file, export a topic, its history or a subtree as
  JSON or CSV, and replay a recording to the connected broker at any speed, optionally under another topic prefix.
- **History on disk** (optional): keep received messages in a local SQLite database so they are back after a
  restart, with limits by age and size.
- Light and dark theme.

## Build and run

You need a [Rust toolchain](https://rustup.rs) (1.100 or newer).

```sh
cargo run --release -p mqtt-viewer
```

Platform notes:

- **macOS**: nothing else is needed.
- **Linux**: install the development packages for xkbcommon, Wayland and XCB, e.g. on Debian/Ubuntu
  `sudo apt install libxkbcommon-dev libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`.
  File dialogs use the XDG desktop portal.
- **Windows**: the MSVC toolchain. The TLS library needs [NASM](https://www.nasm.us) on the `PATH`, or set
  `AWS_LC_SYS_PREBUILT_NASM=1` before building to use its prebuilt objects instead.

### Try it without a broker

The repository contains a small local broker and a load generator:

```sh
cargo run -p mqtt-viewer-core --example broker            # MQTT on 127.0.0.1:1883, MQTT 5 on 1884, WebSocket on 1885
cargo run --release -p mqtt-viewer-core --example flood   # 50 000 topics at 5 000 msg/s for 30 s
```

That broker closes connections that subscribe to `$SYS/#`. The app notices, carries on without that subscription
and tells you; removing the entry from the profile avoids the two-second delay. The broker exists for testing; use
a real one such as Mosquitto for anything else.

## Configuration

Profiles and settings live in one file:

| Platform | Location |
|---|---|
| macOS | `~/Library/Application Support/bircni.mqtt-viewer/config.toml` |
| Linux | `~/.config/mqtt-viewer/config.toml` |
| Windows | `%APPDATA%\bircni\mqtt-viewer\config\config.toml` |

**Passwords are stored in this file in clear text.** The file is created readable by your user only on macOS and
Linux, but anything running as you can read it. Leave the password empty in the profile if that is not acceptable.

The optional history database (`history.sqlite`) is kept in the platform's data directory. Set the environment
variable `MQTT_VIEWER_DATA_DIR` to keep both files in a directory of your choice instead.

If the configuration file cannot be parsed, the app starts with defaults and keeps the unreadable file next to it
as `config.toml.broken`.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check && cargo shear && typos
cargo tarpaulin --workspace --all-features --exclude-files 'apps/mqtt-viewer/src/ui/*' --fail-under 90
```

The workspace has two crates: `crates/mqtt-viewer-core` (connection, topic tree, decoders, recording, history
store; no UI) and `apps/mqtt-viewer` (the egui application). Integration tests start an in-process broker, so no
external services are needed. `cargo test -p mqtt-viewer screenshots -- --ignored` renders the UI to
`target/screenshots/` on a machine with a GPU.

## License

MIT
