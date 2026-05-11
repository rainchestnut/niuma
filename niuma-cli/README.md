# Niuma CLI

[中文说明](README.zh-CN.md)

`niuma` is the Rust desktop gateway for Niuma. The crate lives in
`niuma-cli/`, publishes as the `niuma` crate, and installs the `niuma` binary.

The gateway owns desktop identity, the local pairing dashboard, Niuma Server
registration, Codex app-server process management, mobile task forwarding,
approval and input request forwarding, and local materialization of file
transfers.

## Role

`niuma` is not an AI agent. It is a desktop connector that lets the Niuma iOS
app talk to Codex by starting or connecting to `codex app-server`. Install
Codex.app or the Codex CLI before starting the Niuma gateway.

## Install

1. Install Codex.app or the Codex CLI.
2. Install the desktop gateway from crates.io:

```bash
cargo install niuma
```

From a local checkout, install the same binary with:

```bash
cargo install --path niuma-cli
```

Verify the installed command:

```bash
niuma --help
niuma --version
```

## Repository Scope

`niuma-cli` is one package inside the larger Niuma repository. The repository
root contains the iOS app, the Rust server, and design documents. This package
keeps its own README and LICENSE so the crates.io archive is complete on its
own; the repository root README and LICENSE describe the whole source tree.

## Command Shape

`niuma` uses `clap` and expects a top-level subcommand:

```bash
niuma gateway [OPTIONS]
niuma service <COMMAND>
niuma status [OPTIONS]
niuma reset --yes
```

Rules:

- Top-level commands must be written as `niuma <COMMAND>`.
- Long options use two ASCII hyphens, for example `--help` and `--server-url`.
- Short options use one ASCII hyphen, for example `-h` and `-V`.
- Help belongs at the active command level, for example `niuma gateway --help`.
- Top-level `--help` shows the command list; command-specific options are shown
  by each subcommand.

## Configuration

`niuma gateway` loads configuration in this order:

```text
command-line options > environment variables > ~/.niuma/config.toml > built-in defaults
```

`niuma status` only reads the dashboard host and port, but it follows the same
source order.

Supported `~/.niuma/config.toml` fields:

```toml
server_url = "http://127.0.0.1:8000"
device_name = "My Mac"
dashboard_host = "127.0.0.1"
dashboard_port = 8765
heartbeat_seconds = 30
```

Supported environment variables:

```bash
NIUMA_SERVER_URL=http://127.0.0.1:8000
NIUMA_DEVICE_NAME="My Mac"
NIUMA_DASHBOARD_HOST=127.0.0.1
NIUMA_DASHBOARD_PORT=8765
NIUMA_HEARTBEAT_SECONDS=30
```

Defaults:

- `server_url`: `http://127.0.0.1:8000`
- `dashboard_host`: `127.0.0.1`
- `dashboard_port`: `8765`
- `heartbeat_seconds`: `30`
- `device_name`: `HOSTNAME`, then macOS `scutil --get ComputerName`, then
  `Niuma Desktop`

## `niuma gateway`

Run the complete foreground desktop gateway runtime:

```bash
niuma gateway
```

Startup behavior:

- Creates or reads the desktop identity under `~/.niuma/identity`.
- Registers and authenticates the desktop agent with Niuma Server.
- Starts the local loopback dashboard at `http://127.0.0.1:8765` by default.
- Opens the local pairing page unless disabled.
- Maintains the current QR pairing payload at `/api/pairing/payload`.
- Connects to Codex app-server for mobile tasks, thread sync, approvals, input
  requests, and file transfers.

Common options:

```bash
niuma gateway --server-url https://example.invalid/niuma-server
niuma gateway --dashboard-host 127.0.0.1 --dashboard-port 8765
niuma gateway --device-name "My Mac"
niuma gateway --no-open
niuma gateway --pairing-page-only
niuma gateway --disable-codex-plugins
```

Option meanings:

- `--server-url <URL>`: Niuma Server base URL.
- `--dashboard-host <HOST>`: local dashboard listen host.
- `--dashboard-port <PORT>`: local dashboard listen port.
- `--device-name <NAME>`: desktop agent display name.
- `--no-open`: do not open the browser after startup.
- `--pairing-page-only`: start only the local pairing and diagnostics page,
  without the full Server WebSocket or Codex runtime.
- `--disable-codex-plugins`: pass `--disable plugins` to the spawned Codex
  app-server command.

Use `--pairing-page-only` only when diagnosing the local dashboard or QR pairing
payload. The default `gateway` mode starts the complete runtime.

## `niuma service`

`niuma service` manages a macOS LaunchAgent for the gateway. It does not
currently abstract Linux systemd or Windows Service.

Install the background service:

```bash
niuma service install
```

Install and start immediately:

```bash
niuma service install --start
```

Install a quiet background service:

```bash
niuma service install --no-open
```

`--no-open` is an install-time option written into the LaunchAgent plist. Later
`niuma service start` calls use the installed plist and do not temporarily
change whether the browser opens.

Service lifecycle commands:

```bash
niuma service start
niuma service stop
niuma service restart
niuma service status
niuma service uninstall
```

Behavior:

- `install` writes `~/Library/LaunchAgents/com.niuma.gateway.plist`.
- The plist stores the absolute path to the current `niuma` binary and does not
  depend on background `PATH`.
- `install` does not start by default; use `install --start` or `service start`.
- `start` checks whether the gateway port is already occupied.
- `restart` is equivalent to `stop` followed by `start`.
- `uninstall` stops and removes the LaunchAgent plist.
- `status` prints both launchd state and gateway `/api/status` output or error.

Background logs:

```text
~/.niuma/logs/gateway.out.log
~/.niuma/logs/gateway.err.log
```

## `niuma status`

Read the current gateway HTTP status endpoint:

```bash
niuma status
niuma status --dashboard-host 127.0.0.1 --dashboard-port 8765
```

The command only queries:

```text
http://<dashboard-host>:<dashboard-port>/api/status
```

Use `niuma service status` when launchd state is also needed.

## `niuma reset`

`reset` is destructive and requires explicit confirmation:

```bash
niuma reset --yes
```

It stops and uninstalls the LaunchAgent, then deletes the local Niuma state
directory:

```text
~/.niuma
```

Without `--yes`, the command refuses to run.

## Local State

Runtime state is stored under `~/.niuma`:

```text
~/.niuma/
  config.toml
  identity/
  logs/
  runtime/
  transfers/
```

`niuma-cli` does not migrate the old Python bridge `.niuma-state` directory and
does not read legacy plugin state.

## Common Workflows

Foreground development:

```bash
cargo install --path niuma-cli
niuma gateway
niuma status
```

Background service:

```bash
cargo install niuma
niuma service install --start
niuma service status
niuma service restart
```

Pairing page diagnostics:

```bash
niuma gateway --pairing-page-only
```

Reset local state and pair again:

```bash
niuma reset --yes
niuma gateway
```

## Verification

From the package directory:

```bash
cargo fmt --check
cargo check
cargo test
cargo package --list
cargo publish --dry-run --registry crates-io
```
