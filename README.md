# Codex Fleet

[![CI](https://github.com/142spp/codex-fleet/actions/workflows/ci.yml/badge.svg)](https://github.com/142spp/codex-fleet/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Rust terminal command center for Codex sessions running on multiple computers.
Inspired by `codex agents`: device sections, status filters, search, recent
conversation, Enter to open the original session, and `a` to archive it.

![Synthetic demo of the multi-device dashboard](docs/assets/demo.svg)

The preview uses synthetic sessions. No real session content is included.

## Install and run

```sh
git clone https://github.com/142spp/codex-fleet.git
cd codex-fleet
cargo install --path . --locked
codex-fleet --demo
```

To connect more than the local computer, copy `fleet.example.toml` to a private
configuration file and edit its SSH aliases and optional executable paths:

```sh
mkdir -p ~/.config/codex-fleet
cp fleet.example.toml ~/.config/codex-fleet/fleet.toml
codex-fleet --config ~/.config/codex-fleet/fleet.toml
```

You must already be able to SSH to each host, and each host needs an authenticated
Codex CLI and running shared app-server. Fleet uses those existing sessions;
it does not require a separate OpenAI API key or central service.

### From a checkout

Requires Linux, Rust 1.89 or newer (edition 2024), OpenSSH, and Codex CLI with a running shared
app-server. The current integration targets Codex CLI 0.166.0. Remote hosts may
run different compatible server versions. Codex authentication stays on each host.

```sh
cargo build --locked
cargo run --locked -- --demo
cargo run --locked                       # local shared server only
cp fleet.example.toml fleet.local.toml   # edit SSH aliases and paths
cargo run --locked -- --config fleet.local.toml
```

`fleet.local.toml` is intentionally excluded from Git. Keep host-specific files
and credentials out of commits. Cargo normally installs binaries to `~/.cargo/bin`.

```sh
cargo run --locked -- --config fleet.local.toml --once
cargo run --locked -- --config fleet.local.toml --once --json
cargo run --locked -- --demo --snapshot /tmp/fleet.txt
```

`--once` exits with code 2 if any host fails; JSON still contains each host's
result. `--demo` is synthetic and never connects to a server or opens a session.
Live snapshots can contain private project names and conversation previews.

## Keys

| Key | Action |
| --- | --- |
| Up/Down, j/k, PageUp/PageDown | Navigate sessions |
| Enter / o | Open or resume selected session; exit Codex to return |
| n | New session on selected device; edit its directory, then Enter |
| w | New Git worktree on selected device; edit new branch, then Enter |
| f | Fork selected session and open the copy |
| r | Rename selected session (Ctrl-U clears the name) |
| x | Stop the selected session's active turn |
| a | Archive selected session; running sessions require confirmation |
| h | Hide / unhide selected session locally |
| H | Show hidden sessions / return to normal list |
| Backspace | Permanently delete session after confirmation |
| p | Pin / unpin locally; pinned sessions sort first within each folder |
| Tab / Shift-Tab | Status filter |
| F3 / / | Search title, path, device or thread ID |
| g | Device / project / flat grouping |
| c | Device filter |
| F5 | Refresh / retry connections |
| d | Full recent conversation |
| [ / ] | Scroll the wide-screen detail panel |
| Esc | Close popup, finish search, then clear search |
| ? | Three-column shortcut guide |
| q / Ctrl-C | Quit |

Status is reported by the owning app-server. `Saved` means `notLoaded`, not
completed. `Ready` means loaded and idle. Offline rows are cached and excluded
from live status counts; `All` still includes them. The cache is in memory only.
The default 5-second polling interval means status changes are not instantaneous.

New/forked sessions open in the existing server's Codex CLI without sending a
prompt. New worktrees use the selected repository's committed HEAD, create a new
branch and sibling `.codex-fleet-worktrees` directory, and leave dirty files in
the original checkout. If session creation fails after worktree creation, the
worktree is retained and its path appears in the error message.

Hide and pin are Fleet-local preferences, stored atomically under
`$XDG_STATE_HOME/codex-fleet/state.json` (default `~/.local/state/...`). Use
`--state-file PATH` to select a different preference file. Some tested Codex versions do not expose a compatible pin API, so pinning
deliberately stays local and does not modify server metadata. `H` lets you restore hidden sessions with `h`.
Delete is irreversible and includes spawned descendants; stop an active task
with `x` before deleting it. Stop cancels the turn, not the app-server.
Device sections follow configuration order, with sessions grouped by folder
inside each device. Folders sort by path, and sessions within each folder sort
by recent activity. Device names appear once per section;
project and flat views retain device badges so sessions remain distinguishable.

At 68 columns and above, the list uses aligned Task / Status / Updated columns.
At 110 columns and 14 rows, a right-hand detail panel appears automatically:
model, reasoning, project, branch, latest assistant message and latest prompt.
Selection-triggered reads are debounced by 200 ms and never resume a thread.
Use `[` / `]` to scroll the panel; `d` still opens the full conversation popup.

Archiving keeps the transcript and removes the session from the active list.
Restore it with `codex unarchive <SESSION_ID>` on its original device. The server
may also archive spawned descendants. Running or unknown-status sessions require
`y`/Enter confirmation because archiving may stop their work; `n`/Esc cancels.
Offline sessions cannot be archived. The server status is checked again before
an unconfirmed request; failures leave the session visible.

## Connections and session ownership

Each host has a stable ID, display name, optional SSH alias, Codex executable,
and optional absolute socket path. Without a configured socket, Fleet queries
`codex app-server daemon version` and uses the returned `socketPath` only when
the daemon is already running.

Local connections use WebSockets over the Unix socket. SSH connections run
`codex app-server proxy --sock <path>` and carry the same WebSocket protocol over
stdin/stdout. SSH uses existing OpenSSH configuration, keys and jump hosts;
BatchMode prevents hidden password prompts. No TCP listener or new app-server
is started. Specify `codex` as an absolute remote executable path if necessary.

Enter passes the selected socket explicitly to `codex resume --remote unix://…`
and preserves the session's working directory with `--cd`.
On remote hosts this runs via interactive SSH. The dashboard releases terminal
input and raw mode, lets Codex own the terminal, and restores its view afterward.
Resume never forks a thread or bypasses a writer conflict automatically; only `f` forks. If another
server owns the thread, fix the configured server instead of creating a copy.

Only browsing is read-only. After Enter, the normal Codex CLI can modify the
project when you issue instructions. Fleet does not override Codex model,
approval, sandbox or authentication settings.

## Implementation

- Ratatui + Crossterm: rendering, keyboard input and terminal lifecycle.
- Tokio: one independent connection worker per host; reconnect backoff 1–30s.
- tokio-tungstenite: WebSocket framing, including Unix/SSH streams.
- Serde: tolerant server responses and strict TOML configuration.
- Clap: command-line interface.

The client performs `initialize` / `initialized`, then paginated `thread/list`.
It merges `thread/loaded/list` + `thread/read` so new, unsaved chats also appear.
Historical listing follows Codex's default interactive-source filter; loaded
threads are included regardless of their source.
Details use `thread/read` and `thread/turns/list`. Browsing never sends
`thread/resume`, `turn/start`, approvals or mutations. Explicit shortcuts use
`thread/start`, `thread/fork`, `thread/name/set`, `turn/interrupt`, `thread/archive`
and `thread/delete`, always against the selected device. No prompt is sent by
new-session creation.
Older servers without paginated history still show metadata and the read error.

## Development and tests

```sh
python3 -m venv .venv
.venv/bin/pip install -r requirements-test.txt
```

Install `tmux` using your system package manager for the terminal integration
tests. Run Python commands below from the activated virtual environment.


```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
python3 tests/terminal_smoke.py
python3 tests/shortcuts_smoke.py
```

Tests cover host identity, status classification, Korean search, shell quoting,
pagination, interleaved notifications, request timeout, reconnect/cache behavior,
and rendering at different terminal sizes. The Python PTY test requires `tmux`
and the `websockets` Python package. It uses a mock server and fake CLI only.

An optional real-host test creates a temporary empty chat on each configured
host, enters/exits the actual Codex CLI, checks that no prompt/turn was sent,
archives it through the TUI, verifies it in server archive history, and deletes
only the test chat it created:

```sh
python3 tests/live_navigation.py --config fleet.local.toml
```

Live evidence and known limitations
are recorded in [docs/validation.md](docs/validation.md).

## References

- [OpenAI App Server protocol](https://learn.chatgpt.com/docs/app-server)
- [OpenAI CLI commands and remote mode](https://learn.chatgpt.com/docs/developer-commands)
- [Ratatui terminal handoff recipe](https://ratatui.rs/recipes/apps/spawn-vim/)

## Troubleshooting

If SSH works but Fleet reports a missing socket, check the app-server on that
host with `codex app-server daemon version`. After a reboot, it may need
`codex app-server daemon start`. Fleet intentionally does not start or restart
servers while browsing. If non-interactive SSH cannot find Codex, configure an
absolute `codex` executable path in the host entry.

## License and scope

[MIT](LICENSE). Contributions are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md).
The app-server transport is experimental upstream. This project is an independent
client, not an OpenAI product. Native Windows/macOS, desktop GUI deep links and
in-dashboard model prompts or command approvals are outside the current scope.
