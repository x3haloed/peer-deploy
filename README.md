“push → run everywhere” with hard isolation. Pure WASI on Wasmtime + a p2p Ifrastructure-as-Code.

Architecture (lean, sane)
	•	Agent (Rust): single static binary; runs on any node (bare‑metal, cloud, laptop).
	•	Embeds Wasmtime as the component host.
	•	Pulls/receives signed WASI components and starts them with per‑instance limits.
	•	Exposes a tiny gRPC/HTTP control port (mTLS or Noise).
	•	P2P control plane: libp2p (QUIC + Noise + Kademlia DHT) for discovery and command gossip. No central control server required.
	•	Auth: Ed25519 node keys + Ed25519 owner keys. Owner signs manifests; agents enforce signature + policy. No cloud IAM circus.
	•	IaC state: TOML manifests (amen) describing nodes, components, routes, quotas, secrets. Desired state is replicated via gossip + append‑only log (CRDT or raft-lite).
	•	Resource isolation (per component instance):
	•	Memory cap via Wasmtime max pages.
	•	CPU bound via fuel metering and/or epoch deadlines.
	•	Capabilities via WASI preview2/0.2+ handles (deny by default; selectively allow sockets/http/fs/clock).
	•	Observability:
	•	Agent exposes Prometheus metrics; log drain per component (ring buffer).
	•	One Web UI (SvelteKit or plain HTMX) that subscribes over p2p/pubsub and paints the “kingdom” (nodes, health, utilization, components).

Minimal TOML (desired state)

# realm.toml
[owner]
pubkey = "ed25519:BASE58..."          # who is allowed to push/apply

[nodes."dev-laptop"]
tags = ["dev","darwin","arm64"]
addr_hint = "/dns/dev-laptop.local/udp/443/quic"

[nodes."bm-01"]
tags = ["metal","linux","x86_64"]

[components.auth]
source = "registry:ghcr.io/acme/auth@sha256:..."   # or file:/ s3:/ ipfs:/ p2p:/
world  = "wasi:cli/command"
allowed_outbound = ["https://idp.example.com"]
memory_max_mb = 64
fuel_per_sec = 5_000_000
epoch_timeout_ms = 100
env = { RUST_LOG = "info" }

[components.orders]
source = "file:./build/orders.wasm"
world = "wasi:cli/command"
allowed_outbound = ["postgres://db.internal:5432"]
memory_max_mb = 128
fuel_per_sec = 10_000_000
epoch_timeout_ms = 150
vars = { DB_URL = "postgres://spin@db.internal/shop" }

[routing]
# simple edge mapping handled by the agent's built-in reverse proxy
"bm-01" = [
  { path="/auth/...",   component="auth",   replicas=2 },
  { path="/orders/...", component="orders", replicas=3 }
]

[secrets]
# encrypted-at-rest; delivered as WASI key-value handles
"jwt_pub"   = "age-encrypted:..."
"orders_db" = "age-encrypted:..."

CLI you’ll actually want

# bootstrap a new realm (generates owner key)
realm init
realm key show

# install agent on a node (ssh just once; after that it’s p2p)
ssh bm-01 'curl -sL https://realm.sh/install | sh'

# introduce peers (bootstraps DHT)
realm peer add /dns4/bm-01/tcp/443/quic
realm peer add /dns4/dev-laptop/tcp/443/quic

# register nodes (agents gossip their node keys; you approve)
realm nodes ls
realm nodes approve bm-01 dev-laptop

# deploy desired state
realm apply realm.toml        # signs it with your owner key
realm status                  # live view of drift/health

# ad‑hoc commands (p2p exec)
realm exec bm-01 -- 'ls -la'
realm push orders ./build/orders.wasm
realm restart orders --nodes bm-01

Agent internals (sketch)

Why Rust: easy static binary, first‑class libp2p, best Wasmtime bindings.

// Cargo.toml: wasmtime, wasmtime-wasi, libp2p, quinn, ed25519-dalek, serde, toml, axum(optional)
struct Limits { mem_mb: u64, fuel_per_sec: u64, epoch_ms: u64 }

fn start_component(wasm: &[u8], limits: Limits, permits: WasiPermits) -> anyhow::Result<()> {
    use wasmtime::{Engine, Store, Config, ResourceLimiterAsync};
    let mut cfg = Config::new();
    cfg.wasm_component_model(true)
       .wasm_multi_memory(true)
       .consume_fuel(true);
    let engine = Engine::new(&cfg)?;
    let mut store = Store::new(&engine, ());
    store.add_fuel(limits.fuel_per_sec)?; // top-up strategy on timer

    // epoch deadline preemption
    engine.increment_epoch();
    // … spawn a tokio task that increments epoch periodically; cancel if overtime

    // memory limiter
    struct Lim { max: usize }
    impl ResourceLimiterAsync for Lim {
        fn memory_growing(&mut self, _current: usize, desired: usize, _max: Option<usize>) -> bool {
            desired <= self.max
        }
        fn table_growing(&mut self, _current: u32, _desired: u32, _maximum: Option<u32>) -> bool {
            true
        }
    }
    store.limiter_async(std::sync::Arc::new(tokio::sync::Mutex::new(
        Lim { max: (limits.mem_mb * 1024 * 1024) as usize }
    )));

    // build WASI ctx with explicit capabilities
    let wasi = wasi_cap_std_sync::WasiCtxBuilder::new()
        .inherit_stdin() // or provide pipes
        .inherit_stdout()
        .inherit_stderr()
        // selectively add dirs, sockets, clocks per manifest…
        .build();

    // load component & link wasi:cli/command
    let component = wasmtime::component::Component::from_binary(&engine, wasm)?;
    let mut linker = wasmtime::component::Linker::<()>::new(&engine);
    wasmtime_wasi::add_to_linker_sync(&mut linker, |()| wasi)?;
    let (instance, _bindings) = linker.instantiate(&mut store, &component)?;
    // call the command world’s entrypoint
    // bindings.call_run(&mut store, &[])?;
    Ok(())
}

That’s the heart: load a signed component, apply strict limits, link only the WASI capabilities you allow, run. No Docker.

Security model (simple, strong)
	•	Trust root: your owner public key. Every realm.toml is signed (detached signature beside the file). Agents reject unsigned or stale manifests (monotonic version).
	•	Payload trust: each component reference resolves to a digest‑pinned artifact (sha256:). Agent verifies the digest; optional transparency log (append‑only p2p log of published digests) to detect equivocation.
	•	Node trust: each agent has an Ed25519 node key; first contact is TOFU with owner approval. After that: mutual Noise or mTLS using node certs.
	•	Secrets: sealed to the node key (age or xChaCha20‑Poly1305) and released only to the instance that requests the named handle.

Deployment pipeline (no registries required)
	•	Artifacts: plain files (WASI components). Store anywhere: file:, s3:, ipfs:, p2p:. The agent fetches via a small pluggable fetcher set.
	•	Rollouts:
	•	max_surge, max_unavailable at the manifest route level.
	•	Blue/green with two component labels (orders@blue, orders@green).
	•	Health probe = HTTP handler inside the component or “wasi:cli health” export.

Observability (minimal but useful)
	•	Each instance exports:
	•	wasm_exec_time_ms, wasm_restarts_total
	•	fuel_used_total, epoch_preemptions_total
	•	mem_current_bytes, mem_peak_bytes
	•	Node exports:
	•	CPU %, RAM, disk, p2p links, component counts
	•	UI:
	•	Kingdom map: nodes + tags, latencies, alarms.
	•	Per‑node component table with “restart / scale / drain” buttons.
	•	One‑click tail logs (ring buffer; back‑pressure via SSE).

Selling points
	•	No Docker. No k8s. Just agents + p2p + Wasmtime.
	•	Secure by default: deny‑by‑default capabilities, signed intents, digest‑pinned code.
	•	Easy AF: one binary per node, TOML for desired state, “realm apply”.
	•	Wicked fast: ms startups, tiny memory footprints, precise per‑instance limits.
	•	Portable: any OS/arch where the agent runs.

MVP build plan (ruthlessly small)
	1.	Week 1
	•	Agent: run a single component with mem cap + fuel + epoch.
	•	P2P: libp2p identity + Kademlia + pubsub. “hello world” command gossip.
	•	CLI: realm init, realm apply, realm status.
	2.	Week 2
	•	Signed manifests, digest‑pinned artifacts, file:// and http(s):// fetchers.
	•	Reverse proxy + routing table; scale N replicas per node.
	•	Metrics endpoint + basic TUI/HTML dashboard.
	3.	Stretch
	•	Secrets sealed to node key; rolling update controls; IPFS fetcher.