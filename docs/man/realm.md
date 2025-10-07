## realm

peer-deploy unified agent and CLI.

### Name

realm - peer-to-peer deployment agent and command-line interface

### Overview

The `realm` tool is both:

- a long-running agent that participates in a peer-to-peer network, advertises roles/tags, and executes WASI components and jobs; and
- a CLI for building, packaging, deploying, operating, and observing components across peers.

When invoked without a subcommand, `realm` starts the local agent with the provided global options, joining or discovering the network and enforcing the configured runtime policy.

### Requirements

- Supported OS: Linux and macOS for CLI; agent service install requires a Unix-like system with systemd.
- Networking: UDP/TCP egress to peers; QUIC used for transport.
- Build tooling (optional): `cargo-component` for `deploy-component` workflows.

### Concepts

- Agent: the local daemon that connects to peers and executes work.
- Owner key: a keypair identifying the operator; created with `realm init`.
- Peer: a node in the network, identified by PeerId, optionally tagged with roles.
- Roles/Tags: labels used for placement/targeting (e.g., `edge`, `gpu`, `db`).
- Component: a WASI component (.wasm) with declarative runtime limits and ports.
- Manifest: a TOML desired-state definition including one or more components.
- Package: a portable `.realm` bundle that includes a component and assets.
- Job: an orchestrated task that can produce logs and artifacts.
- Storage (CAS): content-addressed blobs; can be listed, pinned, and GC’d.
- Policy: runtime allow-list for native execution and QEMU emulation.

### Usage

```
realm [OPTIONS] [COMMAND]
```

- Without a subcommand, `realm` launches the agent. Use global options to adjust WASM limits and advertised roles.

### Global Options

- `--wasm <PATH>`: Optional WASM module path to run at startup.
- `--memory-max-mb <INT>`: Maximum memory in MB for WASM. Default: 64.
- `--fuel <INT>`: Initial fuel units for WASM. Default: 0 (unlimited).
- `--epoch-ms <INT>`: Epoch deadline interval in milliseconds. Default: 100.
- `--role <STRING>`: Role/tag to advertise. Repeatable.

### Common Workflows

- Deploy a built WASI component to peers:
  - Build with your toolchain, then: `realm push --name svc --file ./component.wasm --replicas 2 --tag edge`
- Build with cargo-component and deploy in one step:
  - `realm deploy-component --path . --profile release --tag edge --name svc`
- Package a component and static assets for later install:
  - `realm package create --dir ./bundle --output svc.realm`
  - `realm deploy-package --file ./svc.realm`
- Upgrade agents across the fleet:
  - `realm upgrade --bin linux/x86_64=./agent-linux --bin aarch64-apple-darwin=./agent-macos --all-platforms --version 2 --tag edge`
- Operate via the web UI backed by an ephemeral agent:
  - `realm manage --timeout 60`
- Submit and observe jobs:
  - `realm job submit ./job.toml --asset input=./data.bin`
  - `realm job logs <JOB_ID> -f`
- Maintain storage:
  - `realm storage-ls` / `realm storage-pin <DIGEST> --pinned true` / `realm storage-gc 5000000000`

### Commands

- `init`: Generate local owner key.
- `key-show`: Display owner public key.
- `apply`: Send hello / run a WASM / publish a manifest.
- `status`: Query status from agents and print first reply.
- `install`: Install the agent as a service (Unix-only).
- `upgrade`: Push an agent binary upgrade to peers.
- `push`: Push a WASI component to peers and optionally start it.
- `deploy-package`: Deploy a `.realm` package locally.
- `invite`: Create a signed invite token.
- `enroll`: Enroll a new peer using an invite token.
- `configure`: Manually configure trust and bootstrap peers.
- `diag-quic`: Diagnose a QUIC dial to a multiaddr.
- `whoami`: Print identities (CLI owner key, agent trusted owner, agent PeerId).
- `deploy-component`: Build a cargo-component and push to agents.
- `package <SUBCOMMAND>`: Package-related commands.
- `job <SUBCOMMAND>`: Job orchestration commands.
- `p2p <SUBCOMMAND>`: P2P utilities.
- `manage`: Start management web interface.
- `policy-show`: Show current runtime policy (native/QEMU).
- `policy-set`: Set runtime policy flags.
- `storage-ls`: List stored blobs (CAS).
- `storage-pin`: Pin or unpin a blob.
- `storage-gc`: Garbage collect storage to target total size.

See individual pages for detailed options.

### Examples

Run the agent with stricter limits and a UI tag:

```
realm --memory-max-mb 32 --role ui
```

Deploy a component with mounts and a TCP port:

```
realm push --name www \
  --file ./component.wasm \
  --mount host=/var/www,guest=/www,ro=true \
  --port 8080/tcp \
  --replicas 2 --tag edge --start
```

Build and deploy from a Cargo project, targeting specific peers:

```
realm deploy-component --path ./components/www --peer 12D3KooW... --peer 12D3KooX...
```

Pin a blob and garbage-collect storage:

```
realm storage-pin sha256:... --pinned true
realm storage-gc 2147483648
```

### See Also

- `realm-init(1)`, `realm-key-show(1)`, `realm-apply(1)`, `realm-status(1)`, `realm-install(1)`,
  `realm-upgrade(1)`, `realm-push(1)`, `realm-deploy-package(1)`, `realm-invite(1)`,
  `realm-enroll(1)`, `realm-configure(1)`, `realm-diag-quic(1)`, `realm-whoami(1)`,
  `realm-deploy-component(1)`, `realm-manage(1)`, `realm-policy-show(1)`, `realm-policy-set(1)`,
  `realm-storage-ls(1)`, `realm-storage-pin(1)`, `realm-storage-gc(1)`, `realm-package-create(1)`,
  `realm-job-submit(1)`, `realm-job-list(1)`, `realm-job-list-json(1)`, `realm-job-net-list-json(1)`,
  `realm-job-status(1)`, `realm-job-status-json(1)`, `realm-job-net-status-json(1)`,
  `realm-job-cancel(1)`, `realm-job-logs(1)`, `realm-job-artifacts(1)`, `realm-job-download(1)`,
  `realm-job-artifacts-json(1)`, `realm-p2p-watch(1)`
