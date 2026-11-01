# Contributing

Bug reports and focused pull requests are welcome. Include the Fleet and Codex
versions, host operating systems, relevant error text and reproduction steps.
Redact session transcripts, SSH hostnames, addresses and credentials.

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
