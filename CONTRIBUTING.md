# Contributing

## Layout

| Path | What it is |
|---|---|
| `crates/mqtt-viewer-core` | Connection worker, topic tree, decoders, recording, history store. No UI code. |
| `apps/mqtt-viewer` | The egui application. `state.rs` holds all behaviour; `ui/` only draws it. |
| `assets/` | The icon files and what the packages need: the Linux desktop entry and the macOS `Info.plist`. |
| `scripts/` | Release, packaging, icon and screenshot helpers. |
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

CI runs these on Linux; run them before opening a pull request. macOS and Windows are checked on request by the
Platforms workflow (see [Releasing](#releasing)):

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

### Icon

The icon is drawn by `apps/mqtt-viewer/src/icon.rs`. The files in `assets/` are generated from it, and a test
fails while they are outdated. After changing the drawing, run

```sh
scripts/icons.sh
```

A Windows build embeds `assets/icon.ico` and the version into the executable. That needs a resource compiler
(`rc.exe` on Windows, `llvm-rc` when cross-compiling); without one the build still succeeds, with a warning and
without the icon.

## Commits

Use [conventional commits](https://www.conventionalcommits.org) (`feat:`, `fix:`, `ci:` ...). The changelog and
the next version number are derived from them.

## Releasing

Tagging and pushing happen locally; the GitHub release is built in CI when the tag lands.

```sh
scripts/release_version.sh          # or scripts/release_version.ps1 on Windows
scripts/release_version.sh 1.0.0    # to choose the version yourself
```

The script needs [git-cliff](https://git-cliff.org), `cargo-verset` and the GitHub CLI `gh`. It

1. checks that you are on a clean `main` in sync with the remote and that the tag is free,
2. starts the CI, Platforms and Publish workflows on that commit and waits for them, and stops before changing
   anything if one fails (`--skip-checks` leaves this step out),
3. computes the next version from the commits since the last tag, unless one is given,
4. sets it in the workspace `Cargo.toml` and regenerates `CHANGELOG.md`,
5. commits as `release(<version>)`, tags `<version>` and, after asking, pushes `main` and the tag together.

### What the workflows do

| Workflow | Starts | Does |
|---|---|---|
| `ci.yml` | on pushes to `main`, on pull requests, on request | Format, lints, tests, hygiene and coverage, on Linux. |
| `platforms.yml` | on request | Lints and tests on macOS and Windows runners. They cost far more than Linux ones, so they are not part of every push. |
| `publish.yml` | on a version tag, on request | Builds the Linux archive and cross-compiles the Windows one, both on Linux runners. On a tag it also releases them. |

Start one by hand from the Actions tab or with `gh workflow run publish.yml`. Started by hand, Publish builds
and uploads the archives as workflow artifacts and releases nothing, which is the way to try a packaging change.

On a tag, Publish first checks that the tag equals the version in `Cargo.toml`. After the builds it creates a
draft release with the tag's `CHANGELOG.md` section as notes, uploads the archives and a `checksums.txt`, and
only then makes the release public, so nobody finds it half uploaded. After a failure, re-run the failed jobs:
the draft is reused and files with the same names are replaced.

### macOS

The macOS disk image cannot be cross-compiled from Linux, and a macOS runner is not spent on it. Build it on a
Mac from the tagged commit and attach it once the workflow has created the release:

```sh
scripts/bundle_macos.sh
gh release upload <version> dist/mqtt-viewer-<version>-universal-apple-darwin.dmg
```

The script needs both Apple targets (`rustup target add aarch64-apple-darwin x86_64-apple-darwin`). It writes
`dist/MQTT Viewer.app`, a universal binary signed ad hoc, and the disk image next to it.
