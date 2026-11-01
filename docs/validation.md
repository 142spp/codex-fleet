# Validation and compatibility

[← Back to README](../README.md) · [Design](design.md)

## Automated checks

The current local validation includes:

- `cargo fmt --check`
- `cargo clippy --all-targets --locked -- -D warnings`
- `cargo test --locked`: 24 tests
- `cargo build --release --locked`
- `tests/terminal_smoke.py`: a real tmux PTY with a mock Codex server; covers
  Unicode, the responsive panel, normal/error CLI handoffs, archiving and resize.
- `tests/shortcuts_smoke.py`: the task shortcut workflow with mock RPC replies,
  persistent hide/pin state, cancellation and confirmation, and a real disposable
  Git worktree. The original dirty file is preserved.

The GitHub Actions workflow runs the same checks on stable Rust and the declared
minimum Rust version. The workflow uses only synthetic sessions, with no Codex
credentials or access to real computers.

The README demo is generated from the final Ratatui cell buffer. SVG tests cover
foreground/background colors, selection styling, XML escaping, grid placement
and wide Unicode characters. A rasterized render was also visually checked.

## Live integration evidence

Development validation used three Linux computers, including an ARM64 embedded
host, and existing Codex app-server versions 0.164.0, 0.165.0 and 0.166.0. The
checks covered listing, read-only previews, exact-thread CLI entry and return,
local pin/unpin, native fork/delete and archive operations on disposable threads.
The test threads were deleted afterward. No model prompts were sent.

A separate test on app-server 0.166.0 started only `sleep 30` in a temporary
thread and cancelled it using `turn/interrupt`. Its final status was
`interrupted`; the temporary thread was deleted afterward.

The installed 0.166.0 server rejected the documented `isPinned` metadata field,
so Fleet keeps pinning local. These observations are compatibility evidence,
not a guarantee for every Codex release. The upstream transport is experimental.

## Boundaries

- Connection loss/recovery is tested with a mock server. Live SSH routes were
  not deliberately interrupted while they carried real work.
- Real Git worktree creation is tested in disposable local repositories;
  production repositories are not modified during validation.
- Native Windows/macOS builds and desktop GUI navigation have not been tested.
- Private host configurations and live transcript snapshots are excluded from
  the repository. Use the example configuration for your own computers.

## Protocol issues addressed

- The shared Unix socket uses WebSockets, not newline-delimited JSON.
- Loaded but unsaved chats may be absent from history; Fleet merges both lists.
- Terminal restoration avoids cursor-position queries that can time out after
  a child CLI exits.
- CLI handoff explicitly carries the original server socket and directory.
- Responses for an earlier selection cannot overwrite the current preview.

See [design.md](design.md) for the ownership and connection model.
