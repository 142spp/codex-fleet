# Configuration

[← Back to README](../README.md) · [Usage](usage.md)

## Add your devices

Fleet uses your existing OpenSSH configuration and each device's existing Codex
shared app-server. Authenticate Codex on the devices themselves; Fleet needs no
separate OpenAI API key or central service.

Start with the [example configuration](../fleet.example.toml):

```sh
mkdir -p ~/.config/codex-fleet
cp fleet.example.toml ~/.config/codex-fleet/fleet.toml
```

Edit the SSH aliases to match your `~/.ssh/config`:

```toml
poll_seconds = 5
timeout_seconds = 15

[[hosts]]
id = "local"
name = "Laptop"

[[hosts]]
id = "desktop"
name = "Desktop"
ssh = "desktop"

[[hosts]]
id = "jetson"
name = "Jetson"
ssh = "jetson"
```

```sh
codex-fleet --config ~/.config/codex-fleet/fleet.toml
```

The configuration path must be passed explicitly. Without `--config`, Fleet
connects only to the local shared server. Host sections appear in file order.

## Reference

| Setting | Default | Meaning |
| --- | --- | --- |
| `poll_seconds` | `5` | Refresh interval; integer from 1 to 3600 seconds |
| `timeout_seconds` | `15` | Request timeout; integer from 1 to 300 seconds |
| `hosts[].id` | Required | Nonempty, unique host ID; also used for local preferences |
| `hosts[].name` | Required | Nonempty display name |
| `hosts[].ssh` | Omitted | OpenSSH destination/alias; omit for the local device |
| `hosts[].codex` | `"codex"` | Executable name or absolute path on that host |
| `hosts[].socket` | Discovered | Absolute path to an existing app-server socket on that host |

At least one host is required. Unknown configuration fields are rejected.
Keep host IDs stable to retain local pin/hide preferences. Do not put passwords
or private keys in the file. For development checkouts, `fleet.local.toml` is
excluded from Git.

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

## Troubleshooting

### SSH works, but Fleet cannot find Codex

Interactive shells may have a different `PATH`. Set `codex` to the absolute
executable path on the affected host:

```toml
[[hosts]]
id = "desktop"
name = "Desktop"
ssh = "desktop"
codex = "/home/you/.local/bin/codex"
```

### SSH works, but the app-server socket is missing

On the affected device, check the shared server:

```sh
codex app-server daemon version
```

After a reboot, you may need to start it yourself:

```sh
codex app-server daemon start
```

Fleet does not start or restart servers while browsing. If you specify `socket`,
it must point to the server that owns the sessions you intend to use. Press
`F5` to retry after fixing the connection.

### Sessions show Saved, or status updates take a moment

`Saved` means the server has not loaded the thread; it does not mean completed.
Status is polled every 5 seconds by default. Cached sessions remain visible
while a host is offline. See [usage](usage.md) for status and action semantics.

### A Codex version behaves differently

The app-server transport is experimental upstream. See the tested versions and
limits in [validation](validation.md), then include both versions and redacted
error text in a [bug report](https://github.com/142spp/codex-fleet/issues/new?template=bug_report.md).
