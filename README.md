## peer-deploy

“push → run everywhere” with hard isolation. Powered by Wasmtime + WASI and a libp2p control plane.

### What is this?
- **Agent**: a single Rust binary that runs on your nodes. It hosts WASI components with strict resource limits and participates in a P2P control plane.
- **CLI/TUI**: a single Rust binary named `realm` that gives you a text UI and commands to bootstrap, inspect, push components, and upgrade agents.
- **Common protocol**: signed messages over libp2p gossipsub; agents provide a tiny HTTP endpoint for metrics and logs.

## Features
- **WASI components** executed under Wasmtime with memory caps, fuel metering and epoch deadlines.
- **P2P** discovery and command distribution (QUIC + Noise + mDNS + Kademlia).
- **Signed intents**: owner key signs manifests and upgrades; agents enforce signature and TOFU owner trust.
- **Metrics & Logs**: Prometheus metrics and lightweight log tailing served by the agent.
- **Ad‑hoc or desired state**: push a single component, or apply a signed TOML manifest.

## Getting started

### Prerequisites
- Rust toolchain (stable) and `cargo`
- macOS or Linux

### Build
- Build release binaries:
```bash
cargo build --release
```
- Outputs:
  - `target/release/realm` (CLI/TUI)
  - `target/release/agent` (agent)

### Run the TUI
```bash
./target/release/realm
```
- The TUI will open. Use the footer for keybinds.

### Install binaries from the TUI
- Press `I` to install and choose one:
  - **c**: install the CLI/TUI as `realm`
  - **a**: install the Agent as `realm-agent` (you’ll be prompted for the agent binary path)

What the installer does (user-mode on macOS/Linux):
- Copies the binary to a versioned path, maintains a `current` symlink, and places a convenience symlink on your PATH:
  - CLI: `~/.local/bin/realm -> ~/Library/Application Support/realm/bin/current`
  - Agent: `~/.local/bin/realm-agent -> ~/Library/Application Support/realm-agent/bin/current`

Tip for macOS: ensure `~/.local/bin` is on your PATH (zsh):
```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc && source ~/.zshrc
```

### Generate an owner key
```bash
realm init
realm key show   # prints your owner public key (ed25519:BASE58...)
```

### Configure an agent
On a node that runs the agent, trust the owner and optionally add bootstrap peers:
```bash
realm configure --owner <ed25519:BASE58...> --bootstrap \
  /dns4/host.local/udp/443/quic
```
Start the agent (example with tags/roles):
```bash
realm-agent --role dev --role darwin --role arm64
```
The agent exposes metrics and logs on `http://127.0.0.1:9920`.

### Discover and view status
From the TUI, peers discovered via mDNS will show up automatically. Or use the command:
```bash
realm status
```

### Push a WASI component ad‑hoc
The quickest way to try execution on a target peer:
```bash
realm push \
  --name hello \
  --file /path/to/hello.wasm \
  --replicas 1 \
  --memory-max-mb 64 \
  --fuel 5000000 \
  --epoch-ms 100 \
  --tag dev \
  --start
```
- Or from the TUI: press `O` and follow the wizard.
- Selection can target specific peer IDs (`--peer`) or any peers with matching tags (`--tag`).

### Apply a signed manifest
Create a TOML file that lists components and digests (sha256) and apply it:
```bash
realm apply --file ./realm.toml --version 1
```
- The CLI signs the manifest with your owner key.
- Agents verify signature, enforce TOFU on first owner, verify component digests, stage artifacts, and reconcile desired replicas.

### Upgrade agents remotely
- From the TUI: press `U` and provide the path to the new agent binary, optionally targeting peers or tags.
- Or via CLI:
```bash
realm upgrade --file ./target/release/agent --version 2 --tag dev
```
Upgrade behavior on agents:
- Verifies signature on the raw binary bytes and checks owner matches trusted owner
- Verifies sha256 digest
- **Refuses downgrades** (requires higher version than running)
- Writes versioned binary, updates `current` symlink, spawns new process, exits old
- Emits progress to the agent logs so you can observe each phase in the TUI

## Key commands
- **Init owner key**: `realm init`
- **Show owner public key**: `realm key show`
- **Status query**: `realm status`
- **Install from TUI**: press `I` → choose CLI or Agent
- **Push component**: `realm push ...` or `O` in TUI
- **Apply manifest**: `realm apply --file realm.toml --version N`
- **Upgrade agents**: `realm upgrade --file ./agent --version N [--peer ...] [--tag ...]` or `U` in TUI
- **Configure trust/bootstrap on node**: `realm configure --owner <pub> --bootstrap <addr>...`
- **Invite/enroll (optional bootstrap UX)**:
  - Owner: `realm invite --bootstrap <addr> ...` → share token
  - Peer: `realm enroll --token <TOKEN>`

## Agent command‑line (selected)
- `--role <tag>`: repeatable; advertised via libp2p identify
- `--wasm <file>`: start a single WASI component immediately (ad‑hoc)
- `--memory-max-mb`, `--fuel`, `--epoch-ms`: execution limits for ad‑hoc run

## Metrics and logs
- Metrics (Prometheus): `http://127.0.0.1:9920/metrics`
- Logs (plain text): `http://127.0.0.1:9920/logs?component=__all__&tail=200`
- TUI polls these endpoints to render overview tiles and logs.

## Notes & limits
- WASI component should export `run` (command world). If no export is present, the agent will log that and complete without error.
- On macOS, background services are not configured automatically (no systemd). If you want auto-start at login, we can add a `launchd` plist; open an issue.
- The agent’s memory metrics currently report process RSS as a proxy. When Wasmtime exposes per-component stats we’ll switch to those.

## Development
- Build debug:
```bash
cargo build
```
- Build specific crates:
```bash
cargo build -p realm
cargo build -p agent
```

## Security model (short)
- **Trust root**: your owner public key; agents enforce signed messages and TOFU for first owner.
- **Payload trust**: digest‑pinned artifacts (sha256) verified before execution.
- **Transport**: libp2p with Noise; discovery via mDNS and optional bootstrap multiaddrs.

## License
Apache-2.0