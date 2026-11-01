# Usage

[← Back to README](../README.md) · [Configuration](configuration.md)

## Keyboard shortcuts

| Key | Action |
| --- | --- |
| Up/Down, j/k, PageUp/PageDown, Home/End | Navigate sessions |
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

## Status and layout

Status is reported by the owning app-server. `Saved` means `notLoaded`, not
completed. `Ready` means loaded and idle. Offline rows are cached and excluded
from live status counts; `All` still includes them. The cache is in memory only.
The default 5-second polling interval means status changes are not instantaneous.

Device sections follow configuration order, with sessions grouped by folder
inside each device. Folders sort by path, and sessions within each folder sort
by recent activity, with pinned sessions first. Device names appear once per section;
project and flat views retain device badges so sessions remain distinguishable.

At 68 columns and above, the list uses aligned Task / Status / Updated columns.
At 110 columns and 14 rows, a right-hand detail panel appears automatically:
model, reasoning, project, branch, latest assistant message and latest prompt.
Selection-triggered reads are debounced by 200 ms and never resume a thread.
Use `[` / `]` to scroll the panel; `d` still opens the full conversation popup.

## Session actions

### Create, fork and worktree

New/forked sessions open in the existing server's Codex CLI without sending a
prompt. New worktrees use the selected repository's committed HEAD, create a new
branch and sibling `.codex-fleet-worktrees` directory, and leave dirty files in
the original checkout. If session creation fails after worktree creation, the
worktree is retained and its path appears in the error message.

### Pin and hide

Hide and pin are Fleet-local preferences, stored atomically under
`$XDG_STATE_HOME/codex-fleet/state.json` (default
`~/.local/state/codex-fleet/state.json`). Use `--state-file PATH` to select a
different preference file. Some tested Codex versions do not expose a compatible
pin API, so pinning deliberately stays local and does not modify server metadata.
`H` lets you restore hidden sessions with `h`.

### Archive

Archiving keeps the transcript and removes the session from the active list.
Restore it with `codex unarchive <SESSION_ID>` on its original device. The server
may also archive spawned descendants. Running or unknown-status sessions require
`y`/Enter confirmation because archiving may stop their work; `n`/Esc cancels.
Offline sessions cannot be archived. The server status is checked again before
an unconfirmed request; failures leave the session visible.

### Stop and delete

Stop cancels the turn, not the app-server. Delete is irreversible and includes
spawned descendants; stop an active task with `x` before deleting it. Permanent
deletion requires confirmation.

## Command-line tools

Run against the local shared server, or pass a configuration for multiple devices:

```sh
codex-fleet
codex-fleet --config ~/.config/codex-fleet/fleet.toml
```

A configuration file is only loaded when supplied with `--config`.

### Connection checks

```sh
codex-fleet --config ~/.config/codex-fleet/fleet.toml --once
codex-fleet --config ~/.config/codex-fleet/fleet.toml --once --json
```

`--once` exits with code 2 if any host is unavailable. JSON output still includes
each host's result.

### Demo and snapshots

```sh
codex-fleet --demo
codex-fleet --demo --width 140 --height 34 --snapshot /tmp/fleet.svg
codex-fleet --demo --snapshot /tmp/fleet.txt
```

`--demo` uses synthetic data. It does not connect to a server or open a session.
An `.svg` snapshot preserves the rendered terminal's colors and cell positions;
other extensions produce plain text. Live snapshots can contain project names
and conversation previews, so use demo mode for public images.

See `codex-fleet --help` for all command-line options.
