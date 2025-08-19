## peer-deploy

"push → run everywhere" with hard isolation. Powered by Wasmtime + WASI and a libp2p control plane.

### What is this?
- **Agent**: a single Rust binary that runs on your nodes. It hosts WASI components with strict resource limits and participates in a P2P control plane.
- **CLI**: a unified command-line tool named `realm` with subcommands to bootstrap, inspect, push components, and upgrade agents.
- **Web UI**: a management interface served by the agent for deployment, monitoring, and operations via browser.
- **Common protocol**: signed messages over libp2p gossipsub; agents provide HTTP endpoints for metrics, logs, and management.

## Features
- **WASI components** executed under Wasmtime with memory caps, fuel metering and epoch deadlines.
- **P2P** discovery and command distribution (QUIC + Noise + mDNS + Kademlia).
- **Signed intents**: owner key signs manifests and upgrades; agents enforce signature and TOFU owner trust.
- **Metrics & Logs**: Prometheus metrics and lightweight log tailing served by the agent.
- **Web Management**: browser-based UI for deployment, monitoring, operations, and real-time updates.
- **Ad‑hoc or desired state**: push a single component, or apply a signed TOML manifest.
- **Gateway & exposure**: local HTTP gateway on `127.0.0.1:8080` that dispatches HTTP requests into components via the WASI HTTP component model (`wasi:http/incoming-handler`). Optional public bind on peers tagged with `--role edge` when a component requests `visibility=Public`.

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
  - `target/release/realm` (unified CLI)
  - `target/release/agent` (agent binary)

### Quick start
- Run the agent (default command):
```bash
./target/release/realm --role dev
```
- Or launch the web management UI:
```bash
./target/release/realm manage --owner-key <your-key>
```
This starts a temporary node with a random peer ID and ports, avoiding conflicts with a running agent.

### Install binaries
- Build and install via Cargo:
```bash
cargo install --path crates/agent --bin realm
cargo install --path crates/agent --bin agent
```

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
realm whoami     # prints CLI owner pub, agent's trusted owner (if set), and agent PeerId (if running)
```

### Configure an agent
On a node that runs the agent, trust the owner and optionally add bootstrap peers:
```bash
realm configure --owner <ed25519:BASE58...> --bootstrap \
  /dns4/host.local/udp/443/quic
```
Start the agent (example with tags/roles):
```bash
realm --role dev --role darwin --role arm64
```
The agent exposes metrics, logs, and web management on `http://127.0.0.1:9920`. When running `realm manage`, a random high port is chosen to avoid conflicts with a local agent.

On startup, the agent prints a copy‑pastable libp2p multiaddr to stdout, for example:

```
Agent listen multiaddr: /ip4/0.0.0.0/udp/12345/quic-v1/p2p/12D3KooW...
```
The agent now persists its chosen UDP listen port in `~/.local/share/realm-agent/listen_port` and reuses it on restart so peers can reconnect consistently. To set the port explicitly before starting:
```bash
REALM_LISTEN_PORT=60856 realm configure --owner <ed25519:...>
```

### Discover and view status
From the web UI, peers discovered via mDNS will show up automatically. Or use the command:
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
- Or from the web UI: navigate to Deploy and use the form.
- Selection can target specific peer IDs (`--peer`) or any peers with matching tags (`--tag`).

#### Expose a web app (WASI HTTP)
- Components implement `wasi:http/incoming-handler`; the gateway invokes your component per request.
- Push a component and access it under `http://127.0.0.1:8080/{component}/...`:
```bash
realm push \
  --name hello \
  --file /path/to/hello.wasm \
  --visibility local \
  --tag dev \
  --start
```
Then open `http://127.0.0.1:8080/hello/`.

### Apply a signed manifest
Create a TOML file that lists components and digests (sha256) and apply it:
```bash
realm apply --file ./realm.toml --version 1
```
- The CLI signs the manifest with your owner key.
- Agents verify signature, enforce TOFU on first owner, verify component digests, stage artifacts, and reconcile desired replicas.

### Upgrade agents remotely
- From the web UI: navigate to Ops → Upgrade Agent and upload the new binary.
- Or via CLI:
```bash
# Single platform (let agents sniff compatibility)
realm upgrade --file ./target/release/agent --version 2 --tag dev

# Explicit platform targeting (recommended for multi-arch fleets)
realm upgrade --file ./agent-linux-x86_64 --version 3 --platform linux/x86_64 --tag prod
realm upgrade --file ./agent-macos-arm64  --version 3 --platform macos/aarch64 --tag prod
```
Upgrade behavior on agents:
- Verifies signature on the raw binary bytes and checks owner matches trusted owner
- Verifies sha256 digest
- **Refuses downgrades** (requires higher version than running)
- Verifies target platform via header sniff; optional explicit `target_platform` must match `os/arch`
- Writes versioned binary, updates `current` symlink, spawns new process, exits old
- Emits progress to the agent logs so you can observe each phase in the web UI

### Submit jobs with attachments (Phase 6)
- Attach local files/bundles to a job; they are content-addressed and pre-staged before execution.
- CLI examples:
```bash
# Attach a local tarball (auto-named)
realm job submit build-job.toml --asset workspace.tar.gz

# Explicit name → available as /tmp/assets/src
realm job submit build-job.toml --asset src=workspace.tar.gz

# Reuse artifacts from a previous job without re-uploading
realm job artifacts-json build-peer-deploy-1 | jq
realm job submit build-job.toml --use-artifact build-peer-deploy-1:realm-linux-x86_64
```
- Web UI workflow:
  - Jobs → New → paste/edit Job TOML
  - Add files under “Attachments (optional)”; preview shows `/tmp/assets/<filename>` and sha256
  - Submit; assets are pushed to CAS (inline ≤8 MiB, chunked otherwise), announced via P2P, and pre-staged on target before execution
- Size limits & transport:
  - Inline uploads up to ~8 MiB per message
  - Larger files are sent chunked with reassembly and digest verification on the agent
- Execution behavior:
  - Executors (WASM/Native/QEMU) resolve `execution.pre_stage` as `cas:<sha256> → dest` and write files before starting the process

## Key commands
- **Init owner key**: `realm init`
- **Show owner public key**: `realm key show`
- **Status query**: `realm status`
- **Run agent (default)**: `realm --role dev --memory-max-mb 128`
 - **Launch web UI**: `realm manage --owner-key <key> --timeout 30` (spawns a temporary node with random ports)
- **Push component**: `realm push ...` or use web UI Deploy tab
- **Apply manifest**: `realm apply --file realm.toml --version N` or use web UI Ops tab
- **Upgrade agents**: `realm upgrade --file ./agent --version N [--peer ...] [--tag ...]` or use web UI Ops tab
- **Configure trust/bootstrap on node**: `realm configure --owner <pub> --bootstrap <addr>...`
- **Invite/enroll (optional bootstrap UX)**:
  - Owner: `realm invite --bootstrap <addr> ...` → share token
  - Peer: `realm enroll --token <TOKEN>`

## Packages and Mount Lifecycle
- You can bundle a component and assets into a single `.realm` file and deploy it:
  - Create: `realm package create --dir ./my-web-app --name web-app`
  - Deploy: `realm deploy-package --file ./my-web-app.realm`
- Package manifest supports mount kinds with clear data lifecycle semantics:
  - **static**: read‑only assets from the package, content‑addressed under `artifacts/packages/{digest}/…`; swapped on upgrade.
  - **config**: read‑only initial configuration from the package.
  - **work**: read‑write ephemeral working directory. The agent allocates a unique per‑replica subdirectory under `work/components/{name}/{replica-id}` for isolation and removes it when the replica exits or the component is stopped.
  - **state**: read‑write persistent volume under `state/components/{volume}` that survives restarts/upgrades. Optional `seed` copies data from the package into an empty volume on first install only.
- The Web UI (Deploy tab → “Deploy Package”) lets you upload a `.realm`, inspect its manifest and proposed mounts, and view/manage persistent volumes under Ops → Volumes.

## Agent command‑line options
- `--role <tag>`: repeatable; advertised via libp2p identify
- `--wasm <file>`: start a single WASI component immediately (ad‑hoc)
- `--memory-max-mb`, `--fuel`, `--epoch-ms`: execution limits for ad‑hoc run

## Metrics and logs
- Metrics (Prometheus): `http://127.0.0.1:9920/metrics`
- Logs (plain text): `http://127.0.0.1:9920/logs?component=__all__&tail=200`
- Web UI (management): `http://127.0.0.1:9920/` (when launched via `realm manage`)
- Web UI polls these endpoints to render overview tiles and logs.
  - Gateway metrics included: `gateway_requests_total`, `gateway_errors_total`, `gateway_last_latency_ms`

## Notes & limits
- WASI component should export `run` (command world). If no export is present, the agent will log that and complete without error.
- On macOS, background services are not configured automatically (no systemd). If you want auto-start at login, we can add a `launchd` plist; open an issue.
- The agent’s memory metrics currently report process RSS as a proxy. When Wasmtime exposes per-component stats we’ll switch to those.
- Gateway invokes components via WASI HTTP. TLS termination and reverse-proxy features are planned next.

## Runtime Extensions (Phase 4)
Realm adds optional native and QEMU-emulated job runtimes. These are disabled by default for security. Enable via policy:

Create `policy.json` in the agent data dir (see logs for path, usually `~/.local/share/realm-agent/` on Linux, `~/Library/Application Support/realm-agent/` on macOS):
```
{
  "allow_native_execution": true,
  "allow_emulation": true
}
```

Or set environment variables before starting the agent:
```
REALM_ALLOW_NATIVE_EXECUTION=1 REALM_ALLOW_EMULATION=1 realm
```

## Dynamic Peer Discovery (Phase 6+)
Realm features robust peer discovery that automatically forms and maintains mesh networks:

### Multi-layered Discovery
- **mDNS**: Local network discovery for zero-config local mesh formation
- **Bootstrap Peers**: Static multiaddrs in `~/.local/share/realm-agent/bootstrap.json` for initial connections
- **Gossip-based Peer Exchange (PEX)**: Peers periodically share their known addresses via gossipsub 
- **Kademlia DHT**: Distributed hash table for wide-area peer discovery and routing

### Automatic Mesh Formation
- Agents automatically discover and connect to all available peers 
- Bootstrap addresses are propagated throughout the mesh every 30 seconds
- DHT bootstrap runs every 2 minutes to refresh routing tables
- Newly discovered peers (via mDNS or identify) are added to both gossipsub and Kademlia

### Configuration
No manual configuration required! The system automatically:
- Finds local peers via mDNS
- Connects to configured bootstrap peers
- Shares and learns new peer addresses
- Maintains a full mesh topology

**Bootstrap configuration** (optional):
```bash
# Add a known peer to your bootstrap list
realm configure --bootstrap /ip4/192.168.1.100/tcp/39143/p2p/12D3KooW...
```

The bootstrap address will be shared with all connected peers, creating a self-healing mesh network.