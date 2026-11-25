# Contributing

## Layout

| Path | What it is |
|---|---|
| `crates/mqtt-viewer-core` | Connection worker, topic tree, decoders, recording, history store. No UI code. |
| `apps/mqtt-viewer` | The egui application. `state.rs` holds all behaviour; `ui/` only draws it. |
| `scripts/` | Release and screenshot helpers. |
| `docs/` | Screenshots used by the README. |

## Building

You need a [Rust toolchain](https://rustup.rs) (1.100 or newer).

```sh
cargo run -p mqtt-viewer
```

- **macOS**: nothing else is needed.
- **Linux**: the development packages for xkbcommon, Wayland and XCB. On Debian or Ubuntu:
  `sudo apt install libxkbcommon-dev libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`.
  File dialogs use the XDG desktop portal.
- **Windows**: the MSVC toolchain. The TLS library needs [NASM](https://www.nasm.us) on the `PATH`; without it,
  set `AWS_LC_SYS_PREBUILT_NASM=1` before building.

### Cross-compiling for Windows

From macOS or Linux, with [cargo-xwin](https://github.com/rust-cross/cargo-xwin):

```sh
rustup target add x86_64-pc-windows-msvc
AWS_LC_SYS_PREBUILT_NASM=1 cargo xwin build --release -p mqtt-viewer --target x86_64-pc-windows-msvc
```

The executable ends up in `target/x86_64-pc-windows-msvc/release/mqtt-viewer.exe`.

## Checks

CI runs these on Linux; run them before opening a pull request:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
typos
cargo shear
cargo deny check
cargo tarpaulin --workspace --all-features --exclude-files 'apps/mqtt-viewer/src/ui/*' --fail-under 90
```

The lint set in the workspace `Cargo.toml` is strict on purpose (no `unwrap`, no `expect`, no wildcard enum
matches outside tests). Use `#[expect(..., reason = "...")]` where an exception is justified.

## Tests

- **Unit tests** live in `src/tests/<module>_test.rs` of each crate.
- **Integration tests** (`crates/mqtt-viewer-core/tests/integration`) start an in-process broker on a free port,
  plus a TLS-terminating proxy and a throw-away certificate authority where needed. No external services.
- **UI tests** (`apps/mqtt-viewer/src/tests/ui_test.rs`) drive the real drawing code through its accessibility
  tree with `egui_kittest`. Every input widget needs an accessible name, and every glyph must exist in egui's
  built-in font; the tests fail otherwise.
- **End-to-end tests** (`app_test.rs`) run the whole application against the in-process broker.

File dialogs go through the `FilePicker` trait, so tests answer them with a fixed path instead of opening one.

### Trying things by hand

```sh
cargo run -p mqtt-viewer-core --example broker            # MQTT on 127.0.0.1:1883, MQTT 5 on 1884, WebSocket on 1885
cargo run --release -p mqtt-viewer-core --example flood   # 50 000 topics at 5 000 msg/s for 30 s
```

The example broker closes connections that subscribe to `$SYS/#`. The app notices, carries on without that
subscription and says so.

### Screenshots

```sh
scripts/screenshots.sh
```

renders the README screenshots into `docs/`. It needs a machine with a GPU, which is why the test behind it is
ignored by default. Run it after changing the look of the UI.

## Commits

Use [conventional commits](https://www.conventionalcommits.org) (`feat:`, `fix:`, `ci:` ...). The changelog and
the next version number are derived from them.

## Releasing

```sh
scripts/release_version.sh      # or scripts/release_version.ps1 on Windows
```

The script needs [git-cliff](https://git-cliff.org) and `cargo-verset`. It

1. computes the next version from the commits since the last tag,
2. sets it in the workspace `Cargo.toml`,
3. regenerates `CHANGELOG.md`,
4. commits as `release(<version>)`, tags `<version>` and, after asking, pushes.

The pushed tag starts `.github/workflows/release.yml`, which builds the Linux archive and cross-compiles the
Windows one on Linux runners and attaches both to a GitHub release. There is no macOS archive: it cannot be
cross-compiled from Linux, and macOS runners are deliberately not used.
