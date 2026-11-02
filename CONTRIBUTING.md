# Contributing

[← Back to README](README.md) · [Design](docs/design.md) · [Validation](docs/validation.md)

Bug reports and focused pull requests are welcome. Include the Fleet and Codex
versions, host operating systems, relevant error text and reproduction steps.
Redact session transcripts, SSH hostnames, addresses and credentials.

## Getting started

```sh
git clone https://github.com/142spp/codex-fleet.git
cd codex-fleet
cargo run --locked -- --demo
```

The demo runs without a Codex account or remote devices. For real connections,
copy `fleet.example.toml` to the ignored `fleet.local.toml` and follow the
[configuration guide](docs/configuration.md). Keep pull requests focused, and
update user-facing documentation when behavior changes.

## Local checks

Use Rust 1.89 or newer, Python 3.11 or newer, and tmux. Install the Python test
dependency in an isolated virtual environment:

```sh
python3 -m venv .venv
.venv/bin/pip install -r requirements-test.txt
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --locked
.venv/bin/python tests/terminal_smoke.py
.venv/bin/python tests/shortcuts_smoke.py
cargo build --release --locked
```

The normal test suite uses a mock Codex server and temporary directories. It
must not depend on a developer's real sessions or credentials. The shortcuts
test creates a disposable Git repository and verifies that worktree creation
preserves dirty files in the original checkout.

`tests/live_navigation.py` is an opt-in integration test for your own hosts. It
creates, forks, opens, archives and deletes its own temporary test threads. Do
not run it against hosts you do not own or include private connection settings
in a pull request.

Keep session ownership explicit: listing and previewing must not resume a
thread, restart a server, or implicitly approve an operation. Keep all mutations
behind intentional user actions, and retain confirmation before permanent
deletion or archiving an active task.

Contributions are provided under the repository's MIT license.

## Updating the demo

For UI changes, rebuild and regenerate the README demo with
`python3 scripts/render_demo.py`. The exporter reads the same rendered cell
buffer as the TUI, including colors, modifiers and exact cell positions. Do not
recreate screenshots by styling the plain-text snapshot independently.

To regenerate the animated tour, install ImageMagick 7, FFmpeg and the DejaVu Sans Mono
font in addition to tmux, then run:

```sh
cargo build --locked
python3 scripts/record_demo.py
```

The recorder operates the real `--demo` TUI in its own isolated tmux server and
captures continuously at 20 fps, including each typed character and intermediate
preview state. Rendering happens after capture so encoding cannot delay input.
ANSI colors and cell positions are preserved; a shared GIF palette prevents
static text from shimmering. The GIF shows only the terminal screen, with no
added captions or key overlays.

It uses only Python's standard library and does not connect to real devices.
Use `--frames-dir artifacts/demo-frames` to retain the capture, input timing,
SVGs and PNGs for review. Avoid replacing this with one still per feature:
the animation should show a continuous interaction with natural reading pauses.
