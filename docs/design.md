# Design

## Boundary

Fleet owns the terminal UI and its connections. Each existing Codex app-server
continues owning its sessions, turns, execution processes and workspaces. Fleet
does not start, restart, upgrade or stop those servers.

The application is a Linux Rust binary. A local connection uses an existing Unix
socket. A remote connection uses OpenSSH to run `codex app-server proxy`, which
is a byte proxy, not a JSONL translator. HTTP Upgrade and WebSocket framing are
required on both paths. This follows the [OpenAI protocol](https://learn.chatgpt.com/docs/app-server).

## Data flow

One Tokio worker per configured host owns its connection. It discovers the
existing socket, initializes the protocol and fetches all history pages plus
loaded threads. Successful snapshots replace that host's data atomically.
Errors mark the host offline without deleting its previous snapshot. Reconnects
use a 1, 2, 4, 8, 16, 30-second capped delay; manual refresh bypasses that delay.

The UI owns grouping, search, selection and detail state. Session identity is
`(configured host ID, thread ID)`. A refresh keeps selection when that identity
still exists; ordering is stable for equal timestamps. Status counts exclude
offline cached sessions except for the All count.
The default grouping is device → folder → session. Devices follow configuration
order, folders sort by path within each device, and sessions sort by activity
within each folder. Folder groups are device-scoped even when paths match.
Device badges are omitted on these nested rows; other grouping modes retain them.

The responsive layout uses aligned status/time columns from 68 columns, and a
64/36 list/detail split from 110 columns (minimum 14 rows). Narrow terminals keep
the compact list and modal detail view. The side panel reads only the selected
thread, after a 200 ms selection debounce. Responses for previous selections
cannot replace the current panel. Cache invalidation follows the list snapshot's
updatedAt value, avoiding repeated requests when the detail endpoint is newer
than the currently displayed snapshot. Metadata is shown immediately; message
history loads asynchronously through the existing read-only RPC methods.

Archiving is an explicit user action on the selected host/thread identity. The
worker rechecks live status and sends `thread/archive`. Active or unknown-status
sessions require confirmation; no explicit `turn/interrupt` is sent. Duplicate
requests are suppressed while pending. Only a successful response removes the
row, and the following snapshot refresh reconciles server-side descendant
archives. Errors preserve the row and display the server response.

Requests are sequential per connection. Each request has a bounded timeout.
Unrelated server notifications are consumed; live status is obtained by polling
without resuming/subscribing to every historical thread. Unexpected server RPC
requests receive a method-not-found error rather than implicit approval.

## Terminal ownership

Crossterm input polling occurs only on the UI task. Before starting Codex, Fleet
leaves raw/alternate-screen mode and stops reading terminal input. The child
inherits the terminal. After it exits, Fleet restores raw mode and the viewport,
then resumes input polling. Fullscreen redraw uses resize rather than a cursor
position query, which proved unreliable after an actual Codex handoff.

No shell is used for local commands. SSH requires a remote shell command, so each
argument is POSIX-quoted independently. The launcher always passes the owning
server socket and recorded working directory; resume never copies/forks the session
or selects a replacement server. The explicit Fork action creates a new thread. Terminal control bytes in transcript previews
are replaced before rendering.

## Version assumptions and scope

The WebSocket app-server transport is experimental upstream. Fleet was built
against the protocol emitted by CLI 0.166.0 and checked against existing servers
0.164.0, 0.165.0 and 0.166.0. No protocol bindings are assumed to be version-stable.
Unknown status types are shown as Unknown; unsupported history requests leave
the metadata visible with an explanatory error.

First-version exclusions: Windows-native transport, browser UI, GUI window
focus/deep links and in-dashboard prompt/approval
handling, persistent history cache and public-network listeners.

## Task shortcuts

All task mutations are queued on the owning host worker. New and fork responses
insert the returned thread and schedule a CLI handoff to its exact server/socket.
Worktree creation runs Git on the chosen host, based on committed HEAD in a new
sibling directory. It never resets or stashes the original worktree. Name changes
use thread/name/set; stop discovers an in-progress turn before turn/interrupt;
delete rechecks that the task is not active and requires confirmation in the UI.

Hide and pin are local persistent preferences keyed by host/thread, separate
from server history. Pinning uses this local store because the installed 0.166
server rejects the documented isPinned metadata update. H shows hidden tasks;
h restores them. Atomic state-file replacement occurs before changing the UI.
Existing state files with only hidden entries remain readable.
