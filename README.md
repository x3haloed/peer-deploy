# REALM

## WARNING vibe-coded prototype
## NOT FOR PRODUCTION USE

"push → run everywhere" with hard isolation and humane ergonamics. If you've ever wanted to just run runs things on your own servers without losing your mind, this is for you.

<img width="1505" height="628" alt="image" src="https://github.com/user-attachments/assets/48d43906-e69e-4e02-b6c3-f33c019507d4" />

I vibe-coded this in about 5 days in the hopes that a Rust dev will re-write this the right way and then we can all live in nirvana. Different features are in different states of working or broken. Just trying to get the ideas out.

## Features
- **Multiplatform** builds and runs on Mac, Linux, and Windows
- **Single binary** stand-along program file that optionally installs iteself with `./realm install`
- **Host services** WASI, qemu, and native processes can all be orchestrated remotely
- **One-off tasks** run any tool on any machine you control
- **P2P control plane** means every deployment server is also an admin server
- **Bundled web interface** makes everything push-button and easy
- **Rich CLI** makes everything scriptable
- **WASI components** executed under Wasmtime with memory caps, fuel metering and epoch deadlines.
- **Ad‑hoc or desired state**: push a single component, or apply a signed TOML manifest.

## Table of Contents
- [Scenarios](#scenarios)
- [Why I made this](#why)
- [Getting Started](#getting-started)
- [Key Commands](#key-commands)
- [Packages and Mount Lifecycle](#packages-and-mount-lifecycle)
- [Agent Command‑Line Options](#agent-commandline-options)
- [Metrics and Logs](#metrics-and-logs)
- [Runtime Extensions](#runtime-extensions)
- [Dynamic Peer Discovery](#dynamic-peer-discovery)

## Scenarios
### Sandboxed WASI Services

Realm can host tiny, self-contained WebAssemly applications on any number of target hosts. Use the `push` command to send the .wasm file to any computer with Realm installed and specify how much resources you'll allow it consume and how many instances should run. See [components/hello](https://github.com/x3haloed/peer-deploy/tree/main/components/hello) for an example of a tiny WASI web server that can be hosted by Realm. This feature is very similar to what [Spin](https://github.com/spinframework/spin) is doing.

Realm can also host QEMU-hosted services and navtive binary daemons. They're just less cool.

### An entire CI/CD pipeline on one computer

In addition to long-lived services, Realm can run one-off tasks through WASI modules, QEMU VMs, and regular old executable binaries. What does this mean? If you have a computer, you have a bulid pipline. Use any scripting language you like to send jobs out to a Realm machine. In my example, I've got one Debian server that sits around waiting for tasks all day. When I want to make new builds of Realm for every OS and architecture, I can just send a tarball of source code over to it through Realm and have it start making binaries.

See [build-job.toml](https://github.com/x3haloed/peer-deploy/blob/main/build-job.toml), [upgrade-job.toml](https://github.com/x3haloed/peer-deploy/blob/main/upgrade-job.toml), and [upgrade-remote.sh](https://github.com/x3haloed/peer-deploy/blob/main/upgrade-remote.sh) for examples.

### Generic Rentable Computing Platform

"Rent" as in "borrow" -- not as in "apartment." Say you have a few machines of various types floating around like a Linux server, a MacBook, a Windows desktop, and a Raspberry Pi. If they all have realm installed, you now have a computing blob that can churn away on virtually any task. Just send a job out to the swarm, and a capable machine with the right specs and available resources will pick it up and run it.

## WHY?
### I love containers. I hate Docker and LXC
We all know the pitch for containers. All of the dependencies and the full environment are shipped together, faster than a VM, deploy to any host. But Docker is so bloated an painful, I'd rather just deploy to a normal VM. LXC, on the otherhand, it's so obtuse that you have to mmorize the [manpage](https://linuxcontainers.org/lxc/manpages//man7/lxc.7.html) and the [LXD manpage](https://manpages.ubuntu.com/manpages/xenial/en/man1/lxd.1.html), and the [chroot manpage](https://man7.org/linux/man-pages/man2/chroot.2.html) and ... well. I gave up on that years ago.

I've fallen in love with WebAssembly. It's very fast and very portable. It's still rough around the edges, but it's a sandboxed VM in a process, which means it's essentially just a program file you can run next to other program files together on one machine (of any architecture!) without all of the InSaNiTy. Realm has first-class support for Wasmtime WASI components to allow for simple, efficient managed deployments of things like small web applications. It's theoretically possible to build .NET and Node servers that would work here as well, though I don't think any of the major web frameworks will work out of the box. 

Realm packs the whole solution in one binary with a built-in admin UI. Still in the concept phase, Realm is rough around the edges, but I hope you can see where I'm trying to go with it.

### I love IaC. I hate Docker Compose, K8s, and Terraform.
Infrstructure as code is a very practical concept in my mind. It's natural to want to merge our infrastructure specifications with the mechanisms to create the infrastructure, such that when we define the infrastructure we want, it gets built out for us procedurally. No reason it shouldn't work, but it's awful. You won't have to search far to find all kinds of complaints about each of these solutions, but for me, it comes down to the obtusity problem again. I don't think that devs should have to decipher a tome in order to use a tool. That's why Realm supports declaritive deployment specifications in TOML, but allows on-the fly adjustments via CLI or web GUI that can be preserved back to the TOML spec. Not only can this save your butt in an emergency, it also helps us learn as we go, and cement our infra into specification as we build it, rather than having to try and work it out the other way around.

The other issue is administration via centralized control planes. Container orchestration typically pushes you so far out of your own systems that it's really tough to reach into a running machine and figure out what's going on. The reason that Realm uses P2P technology is that it enables every deployment node to be an admin interface. Devops professionals are very familiar with just logging into machines and doing work. With Realm, you might have a bunch of different bare-metal machines that host Realm services, you can RDP into one, pop open the web UI, put in your password, and start taking command of your whole deployment system.

### I love CI/CD. I hate Azure DevOps and GitHub Actions.
Getting your software into as many hands as possible as quickly as possible is a reality of modern business. Not only are we expected to ship quick, it also helps us learn what's working about our software and what needs to change. Having code that's automatically built, tested, and released as soon as I check it in is awesome! So why does it make me want to jump off a cliff? Annoying, obtuse formats and expensive computing requirements. It inevitably takes weeks to set up a production grade CI/CD, thanks to the lousy specs, languages, and formats, and of course you have to rent server space from someone to actually run the builds, or else install an agent on a workstation in the office somewhere and forward ports, and make secret keys, and then the agent can't be found by the platforms, and it stole all your disk space, and deployments are down. It goes on forever. Realm can build any program on a computer that you already have access to. Just tell it to do it. And then save it to TOML.

### I love ... nothing about Cloud.
Oh Cloud. Not terrible in concept, but always terrible in execution. (Noticing a pattern?) I don't know why I have to learn the '[az redis](https://learn.microsoft.com/en-us/cli/azure/redis?view=azure-cli-latest)' command syntax provided by the Azure CLI TO USE REDIS **ON AZURE**. Know what I mean? And then of course our SOC 2 requirements mean that our 20-person business needs a 4-hour RTO window in case a pandemic and a volcano hit the same geographic region while a Cloudflare and AWS outage took down our recovery datacenter at the same time, so of course we have to have deployments with *all three* major cloud vendors, meaning I need to learn three sets of specialized commands for one open source platform that we use solely for the purpose of caching blacklisted, expired authentication tokens. I'd rather just rent a good VM from all three and just push our whole deploment to all three of them with Realm and give the clouds a finger.

### If we've come this far, why not the whole enchilada?
After solving all of the problems above, tacking on traditional VMs via QEMU, native native binary support, and one-off tasks is a breeze. Why not  ¯\\_(ツ)_/¯ Plus, with the addition of those extra things, we get the CI/CD-style platform automation features for cheap. I kind of just stubled across it as I was building out the other stuff. Fprget DevOps, Jenkins, GitHub Actions, and all their crazy formats, proprietary lock-in, expensive VMs, rate-limiting... the problems just never end. I keep coming back to this mantra: I have a computer that's powerful enough to do these things--why is every single automation platform and paradigm built around a proprietary format that runs on someone else's machines at someone else's rates?

### Conclusion
We've lost something in computing. That magical feeling of writing code on your own computer for free, pointing it at the internet, and watching people use your stuff. The languages are all free(-ish), but it's impossible to share your work with other people without paying *somebody* a monthly rent. And for that reason, it's becoming harder and harder to *learn* anything. It's hard to learn how to push iOS apps to an iPhone without paying Apple a developer fee. It's hard to learn how to deploy an ASP.NET Core application to Azure without paying for hosting fees, and it's hard to learn IaC when everything requires an account and a signup and a fee. My biggest hope is that Realm enables people to do stuff with their own computers again. The internet is hard and complicated now, and there's no going back. Everything has to available, durable, and responsive. They're all important things -- it makes the world go 'round. But maybe we can meet those needs on our own darn computers again.

## Getting started

### Uing the CLI
The CLI is robust and supports all functionality. I suggest reviewing the [manpage-style docs](https://github.com/x3haloed/peer-deploy/blob/main/docs/man/realm.md).

### Installation and First Use

```bash
cargo run --release -- manage --owner-key --timeout 30
```

### Prerequisites
- Rust toolchain (stable) and `cargo`
- macOS or Linux

### Build
- Build release binaries:
```bash
cargo build --release
```
- Outputs:
  - `target/release/realm` (unified binary)

### Quick start
- Run the agent (default command):
```bash
./target/release/realm --role dev
```
- Or launch the web management UI:
```bash
./target/release/realm manage --owner-key --timeout 30
```
This starts a temporary node with a random peer ID and ports, avoiding conflicts with a running agent.

### Install binaries
What the agent installer does (user-mode on macOS/Linux):
- Copies the agent binary to a versioned path, maintains a `current` symlink, and places a convenience symlink on your PATH.
- User service (default):
  - Binary store: `<data_dir>/realm-agent/bin/realm-agent-<digest16>`
  - Symlink: `<data_dir>/realm-agent/bin/current`
  - PATH link: `~/.local/bin/realm-agent -> <data_dir>/realm-agent/bin/current`
  - Unit: `~/.config/systemd/user/realm-agent.service`
- System service (`--system`):
  - Binary store: `/usr/local/lib/realm-agent/bin/realm-agent-<digest16>`
  - Symlink: `/usr/local/lib/realm-agent/bin/current`
  - PATH link: `/usr/local/bin/realm-agent -> .../current`
  - Unit: `/etc/systemd/system/realm-agent.service`

See: [realm-install](docs/man/realm-install.md)

Tip for macOS: ensure `~/.local/bin` is on your PATH (zsh):
```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc && source ~/.zshrc
```

### Generate an owner key
```bash
realm init
realm key-show   # prints your owner public key (ed25519:BASE58...)
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
The agent exposes metrics and logs on `http://127.0.0.1:9920`. The management web UI is launched separately via `realm manage` and will print a local URL (random high port) to the console.

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
- Components implement `wasi:http/incoming-handler`; the agent’s gateway invokes your component per request.
- Agents always serve a loopback gateway on `http://127.0.0.1:8080`. A public bind on `0.0.0.0:8080` is enabled automatically when at least one component requests `--visibility public` and the node has the `edge` role.
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
- From the web UI: Ops → Upgrade Agent (multipart upload).
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

### Submit jobs with attachments
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
  - Cluster peers periodically gossip job states; `realm job list` shows the same data on any node. Use `--fresh` to request an immediate sync before listing.
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
  - **work**: read‑write ephemeral working directory under `work/components/{name}`. Intended for scratch space; may be cleaned between runs.
  - **state**: read‑write persistent volume under `state/components/{volume}` that survives restarts/upgrades. Optional `seed` copies data from the package into an empty volume on first install only.
- The Web UI (Deploy tab → “Deploy Package”) lets you upload a `.realm`, inspect its manifest and proposed mounts, and view/manage persistent volumes under Ops → Volumes.

## Agent command‑line options
- `--role <tag>`: repeatable; advertised via libp2p identify
- `--wasm <file>`: start a single WASI component immediately (ad‑hoc)
- `--memory-max-mb`, `--fuel`, `--epoch-ms`: execution limits for ad‑hoc run

## Metrics and logs
- Metrics (Prometheus): `http://127.0.0.1:9920/metrics`
- Logs (plain text): `http://127.0.0.1:9920/logs?component=__all__&tail=200`
- Web UI (management): launched via `realm manage` (prints local URL and port)
- Web UI polls these endpoints to render overview tiles and logs.
  - Gateway metrics included: `gateway_requests_total`, `gateway_errors_total`, `gateway_last_latency_ms`

## Notes & limits
- WASI component should export `run` (command world). If no export is present, the agent will log that and complete without error.
- On macOS, background services are not configured automatically (no systemd). If you want auto-start at login, we can add a `launchd` plist; open an issue.
- The agent’s memory metrics currently report process RSS as a proxy. When Wasmtime exposes per-component stats we’ll switch to those.
- Gateway invokes components via WASI HTTP. TLS termination and reverse-proxy features are planned next.

## Runtime Extensions
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

## Dynamic Peer Discovery
Realm features robust peer discovery that automatically forms and maintains mesh networks:

### Multi-layered Discovery
- **mDNS**: Local network discovery for zero-config local mesh formation
- **Bootstrap Peers**: Static multiaddrs in `~/.local/share/realm-agent/bootstrap.json` for initial connections
- **Gossip-based Peer Exchange (PEX)**: Peers periodically share their known addresses via gossipsub
- **Kademlia DHT**: Distributed hash table for wide-area peer discovery and routing

### Automatic Mesh Formation
- Agents automatically discover and connect to available peers.
- Bootstrap and known peer addresses are announced approximately every 60 seconds.
- DHT bootstrap runs every ~120 seconds to refresh routing tables.
- Newly discovered peers (via mDNS or identify) are added to both gossipsub and Kademlia.

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
