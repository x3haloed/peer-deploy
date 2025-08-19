#![allow(clippy::collapsible_match, clippy::double_ended_iterator_last)]

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::{atomic::Ordering, Arc};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;
use rand::{thread_rng, Rng};
use lru::LruCache;
use uuid::Uuid;
// Maximum number of pending broadcasts to track (evict oldest beyond this)
const MAX_PENDING_BROADCASTS: usize = 50000;
// Pending broadcast entry TTL before automatic prune
const PENDING_ENTRY_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

use anyhow::anyhow;
use futures::StreamExt;
use libp2p::{
    gossipsub, identify, identity, kad, mdns,
    noise, yamux, tcp,
    swarm::{Swarm, SwarmEvent},
    Multiaddr, PeerId, SwarmBuilder,
};
use tracing::{info, warn};
use base64::Engine;

use crate::runner::run_wasm_module_with_limits;
use common::{
    deserialize_message, serialize_message, Command, Status, REALM_CMD_TOPIC, REALM_STATUS_TOPIC,
};
use state::{
    load_bootstrap_addrs, load_known_peers, add_known_peer, load_state, load_trusted_owner,
    load_listen_port, save_listen_port,
    load_listen_port_tcp, save_listen_port_tcp,
    update_persistent_manifest_with_component
};

mod handlers;
pub mod storage;
mod jobs;
mod jobs_wasm;
mod jobs_native;
mod jobs_qemu;
pub mod metrics;
pub mod state;  // Make state module public
mod gateway;

use handlers::{handle_apply_manifest, handle_upgrade};
use jobs::{execute_oneshot_job_with_broadcast, execute_service_job};

struct PendingJob {
    cmd: Command,
    peers: HashSet<PeerId>,
    created: Instant,
    last_sent: Instant,
    retry_count: u32,
}

fn job_status_key(cmd: &Command) -> Option<(String, String, String)> {
    match cmd {
        Command::JobAccepted { job_id, message_id, .. } => Some((job_id.clone(), "accepted".to_string(), message_id.clone())),
        Command::JobStarted { job_id, message_id, .. } => Some((job_id.clone(), "started".to_string(), message_id.clone())),
        Command::JobCompleted { job_id, message_id, .. } => Some((job_id.clone(), "completed".to_string(), message_id.clone())),
        Command::JobFailed { job_id, message_id, .. } => Some((job_id.clone(), "failed".to_string(), message_id.clone())),
        _ => None,
    }
}

fn job_status_tuple(cmd: &Command) -> Option<(String, String, String, String)> {
    match cmd {
        Command::JobAccepted { job_id, assigned_node, message_id } => Some((job_id.clone(), "accepted".to_string(), assigned_node.clone(), message_id.clone())),
        Command::JobStarted { job_id, assigned_node, message_id } => Some((job_id.clone(), "started".to_string(), assigned_node.clone(), message_id.clone())),
        Command::JobCompleted { job_id, assigned_node, message_id, .. } => Some((job_id.clone(), "completed".to_string(), assigned_node.clone(), message_id.clone())),
        Command::JobFailed { job_id, assigned_node, message_id, .. } => Some((job_id.clone(), "failed".to_string(), assigned_node.clone(), message_id.clone())),
        _ => None,
    }
}
use metrics::{push_log, serve_metrics, Metrics, SharedLogs};
use crate::supervisor::Supervisor;


#[derive(libp2p::swarm::NetworkBehaviour)]
struct NodeBehaviour {
    gossipsub: gossipsub::Behaviour,
    kademlia: kad::Behaviour<kad::store::MemoryStore>,
    mdns: mdns::tokio::Behaviour,
    identify: identify::Behaviour,
}

fn load_or_create_node_key() -> identity::Keypair {
    let dir = dirs::data_dir()
        .unwrap_or(std::env::temp_dir())
        .join("realm-agent");
    let path = dir.join("node.key");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(kp) = identity::Keypair::from_protobuf_encoding(&bytes) {
            return kp;
        }
    }
    let kp = identity::Keypair::generate_ed25519();
    if let Ok(enc) = kp.to_protobuf_encoding() {
        let _ = std::fs::write(&path, enc);
    }
    kp
}

// moved to jobs.rs and jobs_wasm.rs

pub async fn run_agent(
    wasm_path: Option<String>,
    memory_max_mb: u64,
    fuel: u64,
    epoch_ms: u64,
    roles: Vec<String>,
    ephemeral: bool,
    // Optional sink to mirror mesh status updates into a shared map (for web UI)
    status_sink: Option<Arc<AsyncMutex<std::collections::BTreeMap<String, common::Status>>>>,
) -> anyhow::Result<()> {
    let metrics = Arc::new(Metrics::new());
    let logs: SharedLogs =
        std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::BTreeMap::new()));
    let sys = std::sync::Arc::new(tokio::sync::Mutex::new(sysinfo::System::new_all()));

    // Start supervisor and restore persistent state
    let supervisor = std::sync::Arc::new(Supervisor::new(logs.clone(), metrics.clone()));
    
    // CRITICAL: Restore persistent component state before starting reconciliation
    if let Err(e) = supervisor.restore_from_disk().await {
        warn!(error=%e, "Failed to restore component state from disk, starting fresh");
    }
    
    // UI/ephemeral nodes should not reconcile desired state or schedule workloads
    if !ephemeral {
        supervisor.clone().spawn_reconcile();
    }

    // Initialize job manager and restore job state
    let job_manager = std::sync::Arc::new(crate::job_manager::JobManager::new(
        state::agent_data_dir().join("jobs")
    ));
    
    if let Err(e) = job_manager.load_from_disk().await {
        warn!(error=%e, "Failed to restore job state from disk, starting fresh");
    }

    let id_keys = if ephemeral {
        identity::Keypair::generate_ed25519()
    } else {
        load_or_create_node_key()
    };
    let local_peer_id = PeerId::from(id_keys.public());

    let gossip_config = gossipsub::ConfigBuilder::default()
        .max_transmit_size(10 * 1024 * 1024) // allow up to 10 MiB messages
        .build()?;

    let mut gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys.clone()),
        gossip_config,
    )
    .map_err(|e| anyhow!(e))?;

    let topic_cmd = gossipsub::IdentTopic::new(REALM_CMD_TOPIC);
    let topic_status = gossipsub::IdentTopic::new(REALM_STATUS_TOPIC);
    gossipsub.subscribe(&topic_cmd)?;
    gossipsub.subscribe(&topic_status)?;

    let store = kad::store::MemoryStore::new(local_peer_id);
    let kademlia = kad::Behaviour::new(local_peer_id, store);

    let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?;

    // Advertise agent version and roles via identify
    let boot = load_state();
    let mut id_cfg = identify::Config::new("peer-deploy/0.1".into(), id_keys.public());
    let roles_str = if roles.is_empty() {
        String::new()
    } else {
        format!(" roles={}", roles.join(","))
    };
    id_cfg.agent_version = format!("realm-agent v{}{}", boot.agent_version, roles_str);
    let identify = identify::Behaviour::new(id_cfg);

    let behaviour = NodeBehaviour {
        gossipsub,
        kademlia,
        mdns,
        identify,
    };

    let mut swarm = SwarmBuilder::with_existing_identity(id_keys.clone())
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_dns()?
        .with_behaviour(|_| Ok(behaviour))?
        .build();

    // Use loopback-only binds for ephemeral UI nodes; persist/reuse public binds for regular agents
    let listen_addr: Multiaddr = if ephemeral {
        "/ip4/127.0.0.1/udp/0/quic-v1".parse()
            .map_err(|e| anyhow!("Failed to parse UDP multiaddr: {}", e))?
    } else if let Some(port) = load_listen_port() {
        format!("/ip4/0.0.0.0/udp/{}/quic-v1", port).parse()
            .map_err(|e| anyhow!("Failed to parse UDP multiaddr: {}", e))?
    } else {
        "/ip4/0.0.0.0/udp/0/quic-v1".parse()
            .map_err(|e| anyhow!("Failed to parse UDP multiaddr: {}", e))?
    };
    Swarm::listen_on(&mut swarm, listen_addr)?;
    // Also listen on TCP to support environments where UDP/QUIC is unavailable; reuse persisted port if any
    let listen_tcp: Multiaddr = if ephemeral {
        "/ip4/127.0.0.1/tcp/0".parse()
            .map_err(|e| anyhow!("Failed to parse TCP multiaddr: {}", e))?
    } else if let Some(port) = load_listen_port_tcp() {
        format!("/ip4/0.0.0.0/tcp/{}", port).parse()
            .map_err(|e| anyhow!("Failed to parse TCP multiaddr: {}", e))?
    } else {
        "/ip4/0.0.0.0/tcp/0".parse()
            .map_err(|e| anyhow!("Failed to parse TCP multiaddr: {}", e))?
    };
    Swarm::listen_on(&mut swarm, listen_tcp)?;
    // Dial configured bootstrap peers and add to Kademlia
    for addr in load_bootstrap_addrs().into_iter() {
        if let Ok(ma) = addr.parse::<Multiaddr>() {
            // Extract PeerId from multiaddr if present for Kademlia
            if let Some(peer_id) = ma.iter().find_map(|p| {
                if let libp2p::multiaddr::Protocol::P2p(hash) = p {
                    PeerId::from_multihash(hash.into()).ok()
                } else { None }
            }) {
                swarm.behaviour_mut().kademlia.add_address(&peer_id, ma.clone());
            }
            let _ = libp2p::Swarm::dial(&mut swarm, ma);
        }
    }
    // Dial persistent known peers from peer store and add to Kademlia
    for addr in load_known_peers().into_iter() {
        if let Ok(ma) = addr.parse::<Multiaddr>() {
            // Extract PeerId from multiaddr if present for Kademlia
            if let Some(peer_id) = ma.iter().find_map(|p| {
                if let libp2p::multiaddr::Protocol::P2p(hash) = p {
                    PeerId::from_multihash(hash.into()).ok()
                } else { None }
            }) {
                swarm.behaviour_mut().kademlia.add_address(&peer_id, ma.clone());
            }
            let _ = libp2p::Swarm::dial(&mut swarm, ma);
        }
    }

    // Track current number of established connections to report in Status
    let mut link_count: usize = 0;

    // channel for run results to publish status from the main loop
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Result<String, String>>();
    
    // channel for job status broadcasts
    let (job_broadcast_tx, mut job_broadcast_rx) = tokio::sync::mpsc::unbounded_channel::<Command>();

    if let Some(path) = wasm_path.clone() {
        let tx0 = tx.clone();
        let logs0 = logs.clone();
        let metrics0 = metrics.clone();
        tokio::spawn(async move {
            push_log(&logs0, "adhoc", format!("starting run {path}")).await;
            let res = run_wasm_module_with_limits(
                &path,
                "adhoc",
                logs0.clone(),
                memory_max_mb,
                fuel,
                epoch_ms,
                Some(metrics0.clone()),
                None,
            )
            .await
            .map(|_| format!("run ok: {path}"))
            .map_err(|e| format!("run error: {e}"));
            match &res {
                Ok(m) => push_log(&logs0, "adhoc", m).await,
                Err(m) => push_log(&logs0, "adhoc", m).await,
            }
            let _ = tx0.send(res);
        });
    }

    // Surface identity and trusted owner for clarity
    let trusted_owner = load_trusted_owner();
    if let Some(owner) = &trusted_owner {
        info!(peer = %local_peer_id, owner_pub = %owner, "agent started");
    } else {
        info!(peer = %local_peer_id, "agent started (no trusted owner yet; TOFU will set on first signed command)");
    }

    // Initialize gauges from persisted state
    let boot_state = load_state();
    metrics.set_manifest_version(boot_state.manifest_version);
    metrics.set_agent_version(boot_state.agent_version);

    if !ephemeral {
        // Spawn metrics server
        tokio::spawn(serve_metrics(
            metrics.clone(),
            logs.clone(),
            "127.0.0.1:9920",
        ));

        // Spawn gateway manager: always serve loopback; add public bind if visibility requires it
        {
            let sup_for_local = supervisor.clone();
            let m = metrics.clone();
            tokio::spawn(async move {
                gateway::serve_gateway(sup_for_local, Some(m), "127.0.0.1:8080").await;
            });
        }
        {
            let sup_for_public = supervisor.clone();
            let metrics_for_public = metrics.clone();
            let roles_for_public = roles.clone();
            tokio::spawn(async move {
                let mut public_spawned = false;
                let mut intv = tokio::time::interval(Duration::from_secs(2));
                loop {
                    intv.tick().await;
                    if !public_spawned {
                        let desired = sup_for_public.get_desired_snapshot().await;
                        let mut any_public = false;
                        for (_name, comp) in desired.iter() {
                            if let Some(vis) = &comp.spec.visibility {
                                if matches!(vis, common::Visibility::Public) {
                                    any_public = true;
                                    break;
                                }
                            }
                        }
                        // gate public binding on 'edge' role present on this peer
                        let is_edge = roles_for_public.iter().any(|r| r == "edge");
                        if any_public && is_edge {
                            // Best effort: start public gateway; if bind fails, log and continue loop
                            let sup2 = sup_for_public.clone();
                            let m2 = metrics_for_public.clone();
                            tokio::spawn(async move {
                                gateway::serve_gateway(sup2, Some(m2), "0.0.0.0:8080").await;
                            });
                            public_spawned = true;
                        }
                    }
                }
            });
        }
    }

    let mut pending_job_broadcasts: HashMap<(String, String, String), PendingJob> = HashMap::new();
    let seen_cache = Arc::new(AsyncMutex::new(LruCache::new(30000)));

    let mut interval = tokio::time::interval(Duration::from_secs(5));
    let mut storage_announce_tick = tokio::time::interval(Duration::from_secs(60));
    let mut peer_announce_tick = tokio::time::interval(Duration::from_secs(60));
    let mut dht_bootstrap_tick = tokio::time::interval(Duration::from_secs(120));
    // Content index: digest -> set of peers that have announced it
    let content_index: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, std::collections::HashSet<String>>>> =
        std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    // Minimal P2P storage request/response plumbing
    let (storage_req_tx, mut storage_req_rx) = tokio::sync::mpsc::unbounded_channel::<storage::StorageRequest>();
    // For reassembling incoming chunked blobs
    let chunk_bufs: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, (u32, Vec<Vec<u8>>)>>>
        = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let pending_storage: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, Vec<tokio::sync::oneshot::Sender<Option<Vec<u8>>>>>>> =
        std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let mut schedule_tick = tokio::time::interval(Duration::from_secs(60));
    // Periodic peer announcement for peer exchange
    // track msgs per second by counting status publishes
    let mut last_publish_count: u64 = 0;
    let mut last_sample_time = std::time::Instant::now();

    loop {
        tokio::select! {
            // Handle job status broadcasts
            Some(job_broadcast) = job_broadcast_rx.recv() => {
                if let Some((job_id, status, message_id)) = job_status_key(&job_broadcast) {
                    let peers: HashSet<PeerId> = swarm.connected_peers().cloned().collect();
                    pending_job_broadcasts.insert(
                        (job_id.clone(), status.clone(), message_id.clone()),
                        PendingJob { cmd: job_broadcast.clone(), peers, created: Instant::now(), last_sent: Instant::now(), retry_count: 0 },
                    );
                }
                let _ = swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), serialize_message(&job_broadcast));
            }
            // Handle incoming run/job/storage events
            Some(run_res) = rx.recv() => {
                match &run_res {
                    Ok(m) => {
                        if m.starts_with("run ok:") { metrics.run_ok_total.fetch_add(1, Ordering::Relaxed); }
                        else if m.starts_with("manifest accepted v") { metrics.manifest_accepted_total.fetch_add(1, Ordering::Relaxed); if let Some(v) = m.split('v').last().and_then(|s| s.split_whitespace().next()).and_then(|s| s.parse::<u64>().ok()) { metrics.set_manifest_version(v); } }
                        else if m.starts_with("upgrade accepted v") { metrics.upgrade_accepted_total.fetch_add(1, Ordering::Relaxed); if let Some(v) = m.split('v').nth(1).and_then(|s| s.split_whitespace().next()).and_then(|s| s.parse::<u64>().ok()) { metrics.set_agent_version(v); } }
                        else { /* generic ok */ }
                    }
                    Err(m) => {
                        if m.starts_with("run error:") { metrics.run_error_total.fetch_add(1, Ordering::Relaxed); }
                        else if m.starts_with("manifest rejected ") { metrics.manifest_rejected_total.fetch_add(1, Ordering::Relaxed); }
                        else if m.starts_with("upgrade rejected ") { metrics.upgrade_rejected_total.fetch_add(1, Ordering::Relaxed); }
                    }
                }
                let msg = match run_res { Ok(m) => m, Err(m) => m };
                let (cpu_percent, mem_percent) = {
                    let mut s = sys.lock().await;
                    s.refresh_all();
                    let cpu = (s.global_cpu_info().cpu_usage() as u64).min(100);
                    let mem = if s.total_memory() == 0 { 0 } else { ((s.used_memory() as f64 / s.total_memory() as f64) * 100.0) as u64 };
                    // Memory reporting note:
                    // - Wasmtime's component API (as of now) does not expose precise per-component
                    //   memory utilization. Until that lands, we report the agent process RSS using sysinfo,
                    //   which is still operationally useful to detect leaks and pressure at the node level.
                    // - When Wasmtime / execution context starts exposing per-store or per-component
                    //   memory usage, we should switch metrics::set_mem_current_bytes() to that data
                    //   (and possibly keep process RSS as a separate metric).
                    if let Some(proc) = s.process(sysinfo::Pid::from_u32(std::process::id())) {
                        let rss_bytes = proc.memory();
                        metrics.set_mem_current_bytes(rss_bytes);
                    }
                    (cpu, mem)
                };
                let status = Status {
                    node_id: local_peer_id.to_string(),
                    msg,
                    agent_version: metrics.agent_version.load(Ordering::Relaxed),
                    components_desired: metrics.components_desired.load(Ordering::Relaxed),
                    components_running: metrics.components_running.load(Ordering::Relaxed),
                    cpu_percent,
                    mem_percent,
                    tags: roles.clone(),
                    drift: metrics.components_desired.load(Ordering::Relaxed) as i64 - metrics.components_running.load(Ordering::Relaxed) as i64,
                    trusted_owner_pub_bs58: load_trusted_owner(),
                    links: link_count as u64,
                };
                // Mirror into shared status sink for UI
                if let Some(sink) = &status_sink { let mut m = sink.lock().await; m.insert(status.node_id.clone(), status.clone()); }
                if let Err(_e) = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), serialize_message(&status)) {
                    metrics.status_publish_errors_total.fetch_add(1, Ordering::Relaxed);
                } else {
                    metrics.status_published_total.fetch_add(1, Ordering::Relaxed);
                    last_publish_count = last_publish_count.saturating_add(1);
                }
            }
            _ = interval.tick() => {
                let (cpu_percent, mem_percent) = {
                    let mut s = sys.lock().await;
                    s.refresh_all();
                    let cpu = (s.global_cpu_info().cpu_usage() as u64).min(100);
                    let mem = if s.total_memory() == 0 { 0 } else { ((s.used_memory() as f64 / s.total_memory() as f64) * 100.0) as u64 };
                    // See note above regarding memory reporting fallback.
                    if let Some(proc) = s.process(sysinfo::Pid::from_u32(std::process::id())) {
                        let rss_bytes = proc.memory();
                        metrics.set_mem_current_bytes(rss_bytes);
                    }
                    (cpu, mem)
                };
                // sample msgs/s
                let elapsed = last_sample_time.elapsed().as_secs_f64();
                if elapsed > 0.0 {
                    let rate = (last_publish_count as f64 / elapsed) as u64;
                    metrics.set_msgs_per_sec(rate);
                    last_publish_count = 0;
                    last_sample_time = std::time::Instant::now();
                }
                let status = Status {
                    node_id: local_peer_id.to_string(),
                    msg: "alive".into(),
                    agent_version: metrics.agent_version.load(Ordering::Relaxed),
                    components_desired: metrics.components_desired.load(Ordering::Relaxed),
                    components_running: metrics.components_running.load(Ordering::Relaxed),
                    cpu_percent,
                    mem_percent,
                    tags: roles.clone(),
                    drift: metrics.components_desired.load(Ordering::Relaxed) as i64 - metrics.components_running.load(Ordering::Relaxed) as i64,
                    trusted_owner_pub_bs58: load_trusted_owner(),
                    links: link_count as u64,
                };
                // Mirror into shared status sink for UI
                if let Some(sink) = &status_sink { let mut m = sink.lock().await; m.insert(status.node_id.clone(), status.clone()); }
                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), serialize_message(&status)) {
                    warn!(error=%e, "failed to publish heartbeat status");
                    metrics.status_publish_errors_total.fetch_add(1, Ordering::Relaxed);
                } else {
                    metrics.status_published_total.fetch_add(1, Ordering::Relaxed);
                    last_publish_count = last_publish_count.saturating_add(1);
                }
                // Prune stale pending broadcasts beyond TTL
                {
                    let now = Instant::now();
                    let stale: Vec<_> = pending_job_broadcasts.iter()
                        .filter_map(|(key, p)| if now.duration_since(p.created) > PENDING_ENTRY_TTL { Some(key.clone()) } else { None })
                        .collect();
                    for key in stale {
                        pending_job_broadcasts.remove(&key);
                        warn!("Pruned stale pending broadcast: {:?}", key);
                    }
                }
                // Evict oldest pending broadcasts if over capacity
                if pending_job_broadcasts.len() > MAX_PENDING_BROADCASTS {
                    let excess = pending_job_broadcasts.len() - MAX_PENDING_BROADCASTS;
                    for _ in 0..excess {
                        if let Some((oldest_key, _)) = pending_job_broadcasts.iter().min_by_key(|(_, p)| p.created) {
                            let key = oldest_key.clone();
                            pending_job_broadcasts.remove(&key);
                            warn!("Evicted pending broadcast due to capacity: {:?}", key);
                        }
                    }
                }
                // Retry pending job broadcasts with exponential backoff and jitter
                for pending in pending_job_broadcasts.values_mut() {
                    // Re-seed peers if none connected
                    if pending.peers.is_empty() {
                        pending.peers = swarm.connected_peers().cloned().collect();
                    }
                    let retry_count = pending.retry_count;
                    let backoff_secs = if retry_count >= 5 { 32 } else { 1 << retry_count };
                    let half = backoff_secs / 2;
                    let mut delay_secs = half + thread_rng().gen_range(0..=half);
                    if delay_secs < 1 {
                        delay_secs = 1;
                    }
                    if !pending.peers.is_empty() && pending.last_sent.elapsed() >= Duration::from_secs(delay_secs) {
                        let _ = swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), serialize_message(&pending.cmd));
                        pending.last_sent = Instant::now();
                        pending.retry_count = pending.retry_count.saturating_add(1);
                    }
                }
            }
            _ = storage_announce_tick.tick() => {
                // Periodically announce a small sample of local blobs
                let store = crate::storage::ContentStore::open();
                // sample up to 8 entries
                let mut count = 0;
                for (digest, entry) in store.list().into_iter() {
                    let _ = swarm.behaviour_mut().gossipsub.publish(
                        topic_status.clone(),
                        serialize_message(&common::Command::StorageHave { digest: digest.clone(), size: entry.size_bytes }),
                    );
                    count += 1;
                    if count >= 8 { break; }
                }
            }
            Some(req) = storage_req_rx.recv() => {
                // Local storage client request: serve from CAS or broadcast StorageGet
                let digest = req.digest.clone();
                let store = crate::storage::ContentStore::open();
                if let Some(path) = store.get_path(&digest) {
                    match tokio::fs::read(path).await {
                        Ok(bytes) => { let _ = req.resp.send(Some(bytes)); }
                        Err(_) => { let _ = req.resp.send(None); }
                    }
                } else {
                    {
                        let mut pend = pending_storage.lock().await;
                        pend.entry(digest.clone()).or_default().push(req.resp);
                    }
                    let _ = swarm.behaviour_mut().gossipsub.publish(
                        topic_cmd.clone(),
                        serialize_message(&common::Command::StorageGet { digest }),
                    );
                }
            }
            _ = schedule_tick.tick() => {
                // Evaluate recurring job schedules
                if let Ok(due_specs) = job_manager.evaluate_schedules().await {
                    for spec in due_specs {
                        // Re-publish the SubmitJob command so eligible nodes can take it
                        let _ = swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), serialize_message(&common::Command::SubmitJob(spec)));
                    }
                }
            }
            // Periodic peer announcement for gossip-based peer exchange
            _ = peer_announce_tick.tick() => {
                // Announce both bootstrap and known peers
                let mut peers = load_bootstrap_addrs();
                for peer in load_known_peers() {
                    if !peers.contains(&peer) {
                        peers.push(peer);
                    }
                }
                if !peers.is_empty() {
                    warn!("Announcing {} peers", peers.len());
                    let msg = Command::AnnouncePeers { peers: peers.clone() };
                    let _ = swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), serialize_message(&msg));
                }
            }
            // Bootstrap Kademlia DHT periodically to maintain routing table
            _ = dht_bootstrap_tick.tick() => {
                if !ephemeral {
                    info!("Bootstrapping Kademlia DHT");
                    let _ = swarm.behaviour_mut().kademlia.bootstrap();
                }
            }
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(ev)) => {
                        match ev {
                            mdns::Event::Discovered(list) => {
                                for (peer, addr) in list { 
                                    // Persist discovered MDNS peer address
                                    add_known_peer(&addr.to_string());
                                    swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer);
                                    // Also add to Kademlia DHT routing table
                                    swarm.behaviour_mut().kademlia.add_address(&peer, addr);
                                }
                            }
                            mdns::Event::Expired(list) => {
                                for (peer, _addr) in list { 
                                    swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer); 
                                    // Note: Kademlia will naturally expire stale entries
                                }
                            }
                        }
                    }
                    SwarmEvent::Behaviour(NodeBehaviourEvent::Kademlia(ev)) => {
                        match ev {
                            kad::Event::RoutingUpdated { peer, .. } => {
                                info!("Kademlia routing updated: {}", peer);
                                // Add to gossipsub for command distribution
                                swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer);
                            }
                            kad::Event::OutboundQueryProgressed { result, .. } => {
                                match result {
                                    kad::QueryResult::Bootstrap(bootstrap_result) => {
                                        match bootstrap_result {
                                            Ok(kad::BootstrapOk { peer, .. }) => {
                                                info!("DHT bootstrap successful via {}", peer);
                                            }
                                            Err(e) => {
                                                warn!("DHT bootstrap error: {:?}", e);
                                            }
                                        }
                                    }
                                    kad::QueryResult::GetClosestPeers(peers_result) => {
                                        match peers_result {
                                            Ok(kad::GetClosestPeersOk { peers, .. }) => {
                                                info!("Found {} closest peers via DHT", peers.len());
                                                for peer_info in peers {
                                                    swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_info.peer_id);
                                                }
                                            }
                                            Err(e) => {
                                                warn!("DHT closest peers query error: {:?}", e);
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                    SwarmEvent::Behaviour(NodeBehaviourEvent::Identify(ev)) => {
                        match ev {
                            identify::Event::Received { peer_id, info, .. } => {
                                info!("Identify received from {}: agent={}", peer_id, info.agent_version);
                                for addr in info.listen_addrs {
                                    // Persist discovered Identify peer address
                                    add_known_peer(&addr.to_string());
                                    // Add peer addresses to Kademlia
                                    swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                                }
                            }
                            identify::Event::Sent { peer_id, .. } => {
                                info!("Identify sent to {}", peer_id);
                            }
                            _ => {}
                        }
                    }
                    SwarmEvent::Behaviour(NodeBehaviourEvent::Gossipsub(ev)) => {
                        if let gossipsub::Event::Message { propagation_source, message_id, message } = ev {
                            // First, try to parse peer Status updates and mirror them into the sink for UI
                            if let Ok(st) = common::deserialize_message::<common::Status>(&message.data) {
                                if let Some(sink) = &status_sink { let mut m = sink.lock().await; m.insert(st.node_id.clone(), st); }
                                continue;
                            }
                            if let Ok(cmd) = deserialize_message::<Command>(&message.data) {
                                // Deduplicate job status messages
                                if let Some((job_id, status, assigned_node, message_id)) = job_status_tuple(&cmd) {
                                    let mut cache = seen_cache.lock().await;
                                    let key = (job_id.clone(), status.clone(), assigned_node.clone());
                                    if cache.contains(&key) {
                                        let ack = Command::JobStatusAck { job_id, status, from: local_peer_id.to_string(), message_id: message_id.clone() };
                                        let _ = swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), serialize_message(&ack));
                                        continue;
                                    }
                                    cache.put(key, Instant::now());
                                }
                                info!(from=%propagation_source, ?message_id, "received command");
                                metrics.commands_received_total.fetch_add(1, Ordering::Relaxed);
                                match cmd {
                                    common::Command::StorageHave { digest, size: _ } => {
                                        // Record in content index
                                        let mut map = content_index.lock().await;
                                        let set = map.entry(digest).or_insert_with(std::collections::HashSet::new);
                                        set.insert(propagation_source.to_string());
                                    }
                                    common::Command::StorageGet { digest } => {
                                        // If we have the blob, and it's not too large, respond inline
                                        let store = crate::storage::ContentStore::open();
                                        if let Some(path) = store.get_path(&digest) {
                                            if let Ok(meta) = tokio::fs::metadata(&path).await {
                                                let size = meta.len();
                                                if size <= 8 * 1024 * 1024 {
                                                    if let Ok(bytes) = tokio::fs::read(&path).await {
                                                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                                                        let _ = swarm.behaviour_mut().gossipsub.publish(
                                                            topic_cmd.clone(),
                                                            serialize_message(&common::Command::StorageData { digest, bytes_b64: b64 }),
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    common::Command::StorageData { digest, bytes_b64 } => {
                                        // Wake up any pending local requests
                                        let mut pend = pending_storage.lock().await;
                                        if let Some(waiters) = pend.remove(&digest) {
                                            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(bytes_b64.as_bytes()) {
                                                for w in waiters { let _ = w.send(Some(bytes.clone())); }
                                            } else {
                                                for w in waiters { let _ = w.send(None); }
                                            }
                                        }
                                    }
                                    common::Command::StoragePut { digest, bytes_b64 } => {
                                        // Accept small blobs inline and store into CAS if digest matches
                                        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(bytes_b64.as_bytes()) {
                                            let calc = common::sha256_hex(&bytes);
                                            if calc == digest {
                                                let store = crate::storage::ContentStore::open();
                                                let _ = store.put_bytes(&bytes);
                                                // Announce availability
                                                let _ = swarm.behaviour_mut().gossipsub.publish(
                                                    topic_status.clone(),
                                                    serialize_message(&common::Command::StorageHave { digest, size: bytes.len() as u64 }),
                                                );
                                            }
                                        }
                                    }
                                    common::Command::StoragePutChunk { digest, chunk_index, total_chunks, bytes_b64 } => {
                                        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(bytes_b64.as_bytes()) {
                                            let mut map = chunk_bufs.lock().await;
                                            let entry = map.entry(digest.clone()).or_insert_with(|| (total_chunks, vec![Vec::new(); total_chunks as usize]));
                                            entry.0 = total_chunks; // update in case first chunk wasn't
                                            if (chunk_index as usize) < entry.1.len() {
                                                entry.1[chunk_index as usize] = bytes;
                                            }
                                            // Check if complete
                                            let complete = entry.1.iter().all(|c| !c.is_empty());
                                            if complete {
                                                let mut full: Vec<u8> = Vec::new();
                                                for c in entry.1.iter() { full.extend_from_slice(c); }
                                                map.remove(&digest);
                                                let calc = common::sha256_hex(&full);
                                                if calc == digest {
                                                    let store = crate::storage::ContentStore::open();
                                                    let _ = store.put_bytes(&full);
                                                    let _ = swarm.behaviour_mut().gossipsub.publish(
                                                        topic_status.clone(),
                                                        serialize_message(&common::Command::StorageHave { digest, size: full.len() as u64 }),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    Command::Hello { from } => {
                                        let (cpu_percent, mem_percent) = {
                                            let mut s = sys.lock().await;
                                            s.refresh_all();
                                            let cpu = (s.global_cpu_info().cpu_usage() as u64).min(100);
                                            let mem = if s.total_memory() == 0 { 0 } else { ((s.used_memory() as f64 / s.total_memory() as f64) * 100.0) as u64 };
                                            (cpu, mem)
                                        };
                                        let status = Status {
                                            node_id: local_peer_id.to_string(),
                                            msg: format!("hello, {from}"),
                                            agent_version: metrics.agent_version.load(Ordering::Relaxed),
                                            components_desired: metrics.components_desired.load(Ordering::Relaxed),
                                            components_running: metrics.components_running.load(Ordering::Relaxed),
                                            cpu_percent,
                                            mem_percent,
                                            tags: roles.clone(),
                                            drift: metrics.components_desired.load(Ordering::Relaxed) as i64 - metrics.components_running.load(Ordering::Relaxed) as i64,
                                            trusted_owner_pub_bs58: load_trusted_owner(),
                                            links: link_count as u64,
                                        };
                                        if let Err(_e) = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), serialize_message(&status)) {
                                            metrics.status_publish_errors_total.fetch_add(1, Ordering::Relaxed);
                                        } else {
                                            metrics.status_published_total.fetch_add(1, Ordering::Relaxed);
                                        }
                                    }
                                    Command::Run { wasm_path, memory_max_mb, fuel, epoch_ms } => {
                                        let tx1 = tx.clone();
                                        let logs1 = logs.clone();
                                        let m_run = metrics.clone();
                                        tokio::spawn(async move {
                                            push_log(&logs1, "adhoc", format!("starting run {wasm_path}")).await;
                                            let res = run_wasm_module_with_limits(&wasm_path, "adhoc", logs1.clone(), memory_max_mb, fuel, epoch_ms, Some(m_run.clone()), None).await
                                                .map(|_| format!("run ok: {wasm_path}"))
                                                .map_err(|e| format!("run error: {e}"));
                                            match &res {
                                                Ok(m) => push_log(&logs1, "adhoc", m).await,
                                                Err(m) => push_log(&logs1, "adhoc", m).await,
                                            }
                                            let _ = tx1.send(res);
                                        });
                                    }
                                    Command::ApplyManifest(signed) => {
                                        let tx2 = tx.clone();
                                        let logs2 = logs.clone();
                                        let m2 = metrics.clone();
                                        let sup = supervisor.clone();
                                        tokio::spawn(async move {
                                            push_log(&logs2, "apply", format!("apply v{}", signed.version)).await;
                                            handle_apply_manifest(tx2, signed, logs2, m2, sup).await;
                                        });
                                    }
                                    Command::UpgradeAgent(pkg) => {
                                        // Selection: peer IDs or tags
                                        let sel_ids = &pkg.target_peer_ids;
                                        let sel_tags = &pkg.target_tags;
                                        let mut selected = true;
                                        if !sel_ids.is_empty() {
                                            selected = sel_ids.iter().any(|s| s == &local_peer_id.to_string());
                                        }
                                        if selected && !sel_tags.is_empty() {
                                            selected = sel_tags.iter().any(|t| roles.contains(t));
                                        }
                                        // Optional platform prefilter: if pkg specifies a target platform, pre-check against host
                                        if selected {
                                            if let Some(ref plat) = pkg.target_platform {
                                                // normalize rust target triples to our os/arch strings
                                                let host = format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH);
                                                if &host != plat {
                                                    selected = false;
                                                }
                                            }
                                        }
                                        if selected {
                                            let tx3 = tx.clone();
                                            let logs3 = logs.clone();
                                            tokio::spawn(async move {
                                                handle_upgrade(tx3, pkg, logs3).await;
                                            });
                                        }
                                    }
                                    Command::StatusQuery => {
                                        let (cpu_percent, mem_percent) = {
                                            let mut s = sys.lock().await;
                                            s.refresh_all();
                                            let cpu = (s.global_cpu_info().cpu_usage() as u64).min(100);
                                            let mem = if s.total_memory() == 0 { 0 } else { ((s.used_memory() as f64 / s.total_memory() as f64) * 100.0) as u64 };
                                            (cpu, mem)
                                        };
                                        let status = Status {
                                            node_id: local_peer_id.to_string(),
                                            msg: "ok".into(),
                                            agent_version: metrics.agent_version.load(Ordering::Relaxed),
                                            components_desired: metrics.components_desired.load(Ordering::Relaxed),
                                            components_running: metrics.components_running.load(Ordering::Relaxed),
                                            cpu_percent,
                                            mem_percent,
                                            tags: roles.clone(),
                                            drift: metrics.components_desired.load(Ordering::Relaxed) as i64 - metrics.components_running.load(Ordering::Relaxed) as i64,
                                            trusted_owner_pub_bs58: load_trusted_owner(),
                                            links: link_count as u64,
                                        };
                                        if let Err(_e) = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), serialize_message(&status)) {
                                            metrics.status_publish_errors_total.fetch_add(1, Ordering::Relaxed);
                                        } else {
                                            metrics.status_published_total.fetch_add(1, Ordering::Relaxed);
                                        }
                                    }
                                    Command::MetricsQuery | Command::LogsQuery { .. } => {
                                        // these are served over HTTP; no-op here for now
                                    }
                                    Command::PushComponent(pkg) => {
                                        info!("Processing PushComponent command for component: {}", pkg.unsigned.component_name);
                                        // Selection: peer IDs or tags
                                        let sel_ids = &pkg.unsigned.target_peer_ids;
                                        let sel_tags = &pkg.unsigned.target_tags;
                                        let mut selected = true;
                                        if !sel_ids.is_empty() {
                                            selected = sel_ids.iter().any(|s| s == &local_peer_id.to_string());
                                            info!("Target peer IDs: {:?}, selected by peer ID: {}", sel_ids, selected);
                                        }
                                        if selected && !sel_tags.is_empty() {
                                            // require at least one common tag
                                            selected = sel_tags.iter().any(|t| roles.contains(t));
                                            info!("Target tags: {:?}, agent tags: {:?}, selected by tag: {}", sel_tags, roles, selected);
                                        }
                                        // Never accept pushes on nodes tagged as UI
                                        if selected && roles.iter().any(|r| r == "ui") {
                                            info!("UI-tagged node: rejecting PushComponent for {}", pkg.unsigned.component_name);
                                            selected = false;
                                        }
                                        info!("Component {} selection result: {}", pkg.unsigned.component_name, selected);
                                        if selected {
                                            // Verify signature and digest
                                            info!("Starting signature verification for component {}", pkg.unsigned.component_name);
                                            if let Ok(unsigned_bytes) = serde_json::to_vec(&pkg.unsigned) {
                                                info!("Serialized unsigned data successfully");
                                                if let Ok(sig_bytes) = base64::engine::general_purpose::STANDARD.decode(&pkg.signature_b64) {
                                                    info!("Decoded signature successfully");
                                                    let sig_valid = common::verify_bytes_ed25519(&pkg.unsigned.owner_pub_bs58, &unsigned_bytes, &sig_bytes).unwrap_or(false);
                                                    info!("Signature verification result: {} for owner: {}", sig_valid, pkg.unsigned.owner_pub_bs58);
                                                    if sig_valid {
                                                        if let Ok(bin_bytes) = base64::engine::general_purpose::STANDARD.decode(&pkg.binary_b64) {
                                                            let digest = common::sha256_hex(&bin_bytes);
                                                            if digest == pkg.unsigned.binary_sha256_hex {
                                                                // TOFU owner trust like other commands
                                                                info!("Binary digest verified successfully");
                                                                if let Some(trusted) = crate::p2p::state::load_trusted_owner() {
                                                                    info!("Found trusted owner: {}, command owner: {}", trusted, pkg.unsigned.owner_pub_bs58);
                                                                    if trusted != pkg.unsigned.owner_pub_bs58 {
                                                                        warn!("push: owner mismatch");
                                                                    } else {
                                                                        info!("Owner verified, staging artifact for component {}", pkg.unsigned.component_name);
                                                                        // Stage artifact
                                                                        let stage_dir = crate::p2p::state::agent_data_dir().join("artifacts");
                                                                        if tokio::fs::create_dir_all(&stage_dir).await.is_ok() {
                                                                            let file_path = stage_dir.join(format!("{}-{}.wasm", pkg.unsigned.component_name, &digest[..16]));
                                                                            if !file_path.exists() {
                                                                                if tokio::fs::write(&file_path, &bin_bytes).await.is_err() {
                                                                                    warn!("push: write failed");
                                                                                }
                                                                            }
                                                                            push_log(&logs, &pkg.unsigned.component_name, format!("pushed {} bytes (sha256={})", bin_bytes.len(), &digest[..16])).await;
                                                                            if pkg.unsigned.start {
                                                                                info!("Component {} marked for start, creating spec and scheduling", pkg.unsigned.component_name);
                                                                                let spec = common::ComponentSpec {
                                                                                    source: format!("cached:{}", pkg.unsigned.binary_sha256_hex.clone()),
                                                                                    sha256_hex: pkg.unsigned.binary_sha256_hex.clone(),
                                                                                    memory_max_mb: pkg.unsigned.memory_max_mb,
                                                                                    fuel: pkg.unsigned.fuel,
                                                                                    epoch_ms: pkg.unsigned.epoch_ms,
                                                                                    replicas: Some(pkg.unsigned.replicas),
                                                                                    mounts: pkg.unsigned.mounts.clone(),
                                                                                    ports: pkg.unsigned.ports.clone(),
                                                                                    visibility: pkg.unsigned.visibility.clone(),
                                                                                };
                                                                                let desired = crate::supervisor::DesiredComponent { name: pkg.unsigned.component_name.clone(), path: file_path.clone(), spec: spec.clone() };
                                                                                info!("Calling supervisor.upsert_component for {}", pkg.unsigned.component_name);
                                                                                supervisor.upsert_component(desired).await;
                                                                                
                                                                                // CRITICAL: Persist the component to manifest for restart persistence
                                                                                update_persistent_manifest_with_component(&pkg.unsigned.component_name, spec);
                                                                                
                                                                                info!("Component {} successfully scheduled and persisted", pkg.unsigned.component_name);
                                                                                push_log(&logs, &pkg.unsigned.component_name, "scheduled (upsert)" ).await;
                                                                            }
                                                                        }
                                                                    }
                                                                } else {
                                                                    // TOFU: accept first signed push and trust this owner, also stage and schedule
                                                                    info!("No trusted owner found, performing TOFU for owner: {}", pkg.unsigned.owner_pub_bs58);
                                                                    crate::p2p::state::save_trusted_owner(&pkg.unsigned.owner_pub_bs58);
                                                                    let stage_dir = crate::p2p::state::agent_data_dir().join("artifacts");
                                                                    if tokio::fs::create_dir_all(&stage_dir).await.is_ok() {
                                                                        let file_path = stage_dir.join(format!("{}-{}.wasm", pkg.unsigned.component_name, &digest[..16]));
                                                                        if !file_path.exists() {
                                                                            if tokio::fs::write(&file_path, &bin_bytes).await.is_err() {
                                                                                warn!("push: write failed (TOFU)");
                                                                            }
                                                                        }
                                                                        push_log(&logs, &pkg.unsigned.component_name, format!("pushed {} bytes (sha256={})", bin_bytes.len(), &digest[..16])).await;
                                                                        if pkg.unsigned.start {
                                                                            let spec = common::ComponentSpec {
                                                                                source: format!("cached:{}", pkg.unsigned.binary_sha256_hex.clone()),
                                                                                sha256_hex: pkg.unsigned.binary_sha256_hex.clone(),
                                                                                memory_max_mb: pkg.unsigned.memory_max_mb,
                                                                                fuel: pkg.unsigned.fuel,
                                                                                epoch_ms: pkg.unsigned.epoch_ms,
                                                                                replicas: Some(pkg.unsigned.replicas),
                                                                                mounts: pkg.unsigned.mounts.clone(),
                                                                                ports: pkg.unsigned.ports.clone(),
                                                                                visibility: pkg.unsigned.visibility.clone(),
                                                                            };
                                                                            let desired = crate::supervisor::DesiredComponent { name: pkg.unsigned.component_name.clone(), path: file_path.clone(), spec: spec.clone() };
                                                                            supervisor.upsert_component(desired).await;
                                                                            
                                                                            // CRITICAL: Persist the component to manifest for restart persistence
                                                                            update_persistent_manifest_with_component(&pkg.unsigned.component_name, spec);
                                                                            
                                                                            push_log(&logs, &pkg.unsigned.component_name, "scheduled (upsert)" ).await;
                                                                        }
                                                                    }
                                                                }
                                                            } else {
                                                                warn!("push: digest mismatch");
                                                            }
                                                        } else {
                                                            warn!("push: bad binary_b64");
                                                        }
                                                    } else {
                                                        warn!("push: signature verify failed");
                                                    }
                                                } else {
                                                    warn!("push: bad signature_b64");
                                                }
                                            } else {
                                                warn!("push: unsigned serialize err");
                                            }
                                        }
                                    }
                                    Command::SubmitJob(job) => {
                                        let txj = tx.clone();
                                        let logsj = logs.clone();
                                        let rolesj = roles.clone();
                                        let job_mgr = job_manager.clone();
                                        let node_id = local_peer_id.to_string();
                                        let content_index2 = content_index.clone();
                                        let storage_tx = storage_req_tx.clone();
                                        let job_broadcast_tx_clone = job_broadcast_tx.clone();
                                        tokio::spawn(async move {
                                            // Check if this node is eligible to execute the job
                                            let mut eligible = true;
                                            if let Some(t) = &job.targeting {
                                                if let Some(p) = &t.platform {
                                                    let host = format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH);
                                                    if &host != p { eligible = false; }
                                                }
                                                if eligible && !t.tags.is_empty() {
                                                    eligible = t.tags.iter().any(|tag| rolesj.contains(tag));
                                                }
                                                if eligible && !t.node_ids.is_empty() {
                                                    eligible = t.node_ids.iter().any(|id| id == &node_id);
                                                }
                                            }
                                            
                                            if !eligible {
                                                let _ = push_log(&logsj, "system", format!("Job {} not eligible for this node, ignoring", job.name)).await;
                                                return;
                                            }
                                            
                                            // This node is eligible - check if job is already accepted by another node
                                            if let Some(existing) = job_mgr.get_job(&format!("{}-{}", job.name, "pending")).await {
                                                if existing.assigned_node.is_some() {
                                                    let _ = push_log(&logsj, "system", format!("Job {} already accepted by another node", job.name)).await;
                                                    return;
                                                }
                                            }
                                            
                                            // Accept the job and broadcast acceptance
                                            let job_id = match job_mgr.submit_job(job.clone()).await {
                                                Ok(id) => id,
                                                Err(e) => {
                                                    let _ = txj.send(Err(format!("job submission failed: {}", e)));
                                                    return;
                                                }
                                            };
                                            
                                            // Mark this node as the assigned executor
                                            let _ = job_mgr.assign_job(&job_id, &node_id).await;
                                            
                                            // Broadcast job acceptance to prevent other nodes from taking it
                                            let acceptance_msg = Command::JobAccepted { 
                                                job_id: job_id.clone(), 
                                                assigned_node: node_id.clone(), 
                                                message_id: Uuid::new_v4().to_string(),
                                            };
                                            let _ = job_broadcast_tx_clone.send(acceptance_msg);
                                            
                                            let _ = push_log(&logsj, "system", format!("Job accepted: {} ({})", job.name, job_id)).await;
                                            
                                            // This node is executing the job
                                            let selected = true;
                                            
                                            // Locality preference: if job has digest and others have it, delay start here
                                            let mut locality_delay_ms: u64 = 0;
                                            let job_digest: Option<String> = match &job.runtime {
                                                common::JobRuntime::Wasm { sha256_hex, .. } => sha256_hex.clone(),
                                                common::JobRuntime::Native { sha256_hex, .. } => sha256_hex.clone(),
                                                common::JobRuntime::Qemu { sha256_hex, .. } => sha256_hex.clone(),
                                            };
                                            if let Some(d) = &job_digest {
                                                let store = crate::storage::ContentStore::open();
                                                let has_local = store.has(d);
                                                if !has_local {
                                                    let peers_with = {
                                                        let map = content_index2.lock().await;
                                                        map.get(d).map(|s| s.len()).unwrap_or(0)
                                                    };
                                                    if peers_with > 0 {
                                                        let h = common::sha256_hex(node_id.as_bytes());
                                                        let nib = u64::from_str_radix(&h[..4], 16).unwrap_or(0);
                                                        locality_delay_ms = 500 + (nib % 1500);
                                                    }
                                                }
                                            }
                                            if selected {
                                                // Mark job as started
                                                let _ = job_mgr.start_job(&job_id).await;
                                                let _ = job_mgr.add_job_log(&job_id, "info".to_string(), "Job execution started on this node".to_string()).await;
                                                
                                                // Broadcast job started status
                                                let start_msg = Command::JobStarted { 
                                                    job_id: job_id.clone(), 
                                                    assigned_node: node_id.clone(), 
                                                    message_id: Uuid::new_v4().to_string(),
                                                };
                                                let _ = job_broadcast_tx_clone.send(start_msg);
                                                
                                                // Observability: log start
                                                let _ = push_log(&logsj, "system", format!("job started: {}", job_id)).await;
                                                
                                                // Handle different job types
                                                match &job.job_type {
                                                    common::JobType::OneShot => {
                                                        if locality_delay_ms > 0 { tokio::time::sleep(Duration::from_millis(locality_delay_ms)).await; }
                                                        execute_oneshot_job_with_broadcast(job_mgr.clone(), job_id.clone(), job.clone(), logsj.clone(), txj.clone(), Some(storage::P2PStorage::new(storage_tx.clone())), job_broadcast_tx_clone.clone(), node_id.clone()).await;
                                                    },
                                                    common::JobType::Recurring => {
                                                        // Recurring jobs are handled by the scheduler, treat execution as one-shot
                                                        if locality_delay_ms > 0 { tokio::time::sleep(Duration::from_millis(locality_delay_ms)).await; }
                                                        execute_oneshot_job_with_broadcast(job_mgr.clone(), job_id.clone(), job.clone(), logsj.clone(), txj.clone(), Some(storage::P2PStorage::new(storage_tx.clone())), job_broadcast_tx_clone.clone(), node_id.clone()).await;
                                                    },
                                                    common::JobType::Service => {
                                                        // Create cancellation channel for service jobs
                                                        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
                                                        
                                                        let service_job_mgr = job_mgr.clone();
                                                        let service_job_id = job_id.clone();
                                                        let service_job = job.clone();
                                                        let service_logs = logsj.clone();
                                                        let service_tx = txj.clone();
                                                        
                                                        let handle = tokio::spawn(async move {
                                                            if locality_delay_ms > 0 { tokio::time::sleep(Duration::from_millis(locality_delay_ms)).await; }
                                                            execute_service_job(service_job_mgr, service_job_id, service_job, service_logs, service_tx, cancel_rx, Some(storage::P2PStorage::new(storage_tx.clone()))).await;
                                                        });
                                                        
                                                        // Register the running job for cancellation support
                                                        job_mgr.register_running_job(job_id.clone(), handle, cancel_tx).await;
                                                    }
                                                }
                                            } else {
                                                // Job not selected for this node, but still track it
                                                let _ = job_mgr.add_job_log(&job_id, "info".to_string(), "Job not selected for execution on this node".to_string()).await;
                                            }
                                        });
                                    }
                                    Command::QueryJobs { status_filter, limit } => {
                                        let jobs = job_manager.list_jobs(status_filter.as_deref(), limit).await;
                                        // Publish the job list as a response
                                        let response = serialize_message(&jobs);
                                        let _ = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), response);
                                    }
                                    Command::QueryJobStatus { job_id } => {
                                        if let Some(job) = job_manager.get_job(&job_id).await {
                                            let response = serialize_message(&job);
                                            let _ = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), response);
                                        }
                                    }
                                    Command::CancelJob { job_id } => {
                                        let _ = job_manager.cancel_job(&job_id).await;
                                    }
                                    Command::QueryJobLogs { job_id, tail: _ } => {
                                        if let Some(job) = job_manager.get_job(&job_id).await {
                                            let response = serialize_message(&job);
                                            let _ = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), response);
                                        }
                                    }
                                    Command::JobAccepted { job_id, assigned_node, message_id } => {
                                        // Another node accepted this job - mark it in our local job store
                                        if let Some(mut job) = job_manager.get_job(&job_id).await {
                                            job.assigned_node = Some(assigned_node.clone());
                                            let _ = job_manager.update_job_assignment(&job_id, &assigned_node).await;
                                            let _ = push_log(&logs, "system", format!("Job {} accepted by node {}", job_id, assigned_node)).await;
                                        }
                                        let ack = Command::JobStatusAck { job_id: job_id.clone(), status: "accepted".to_string(), from: local_peer_id.to_string(), message_id: message_id.clone() };
                                        let _ = swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), serialize_message(&ack));
                                    }
                                    Command::JobStarted { job_id, assigned_node, message_id } => {
                                        let _ = job_manager.start_job(&job_id).await;
                                        let _ = push_log(&logs, "system", format!("Job {} started on node {}", job_id, assigned_node)).await;
                                        let ack = Command::JobStatusAck { job_id: job_id.clone(), status: "started".to_string(), from: local_peer_id.to_string(), message_id: message_id.clone() };
                                        let _ = swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), serialize_message(&ack));
                                    }
                                    Command::JobCompleted { job_id, assigned_node, exit_code, message_id } => {
                                        let _ = job_manager.complete_job(&job_id, exit_code).await;
                                        let _ = push_log(&logs, "system", format!("Job {} completed on node {} with exit code {}", job_id, assigned_node, exit_code)).await;
                                        let ack = Command::JobStatusAck { job_id: job_id.clone(), status: "completed".to_string(), from: local_peer_id.to_string(), message_id: message_id.clone() };
                                        let _ = swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), serialize_message(&ack));
                                    }
                                    Command::JobFailed { job_id, assigned_node, error, message_id } => {
                                        let _ = job_manager.fail_job(&job_id, error.clone()).await;
                                        let _ = push_log(&logs, "system", format!("Job {} failed on node {}: {}", job_id, assigned_node, error)).await;
                                        let ack = Command::JobStatusAck { job_id: job_id.clone(), status: "failed".to_string(), from: local_peer_id.to_string(), message_id: message_id.clone() };
                                        let _ = swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), serialize_message(&ack));
                                    }
                                    Command::JobStatusAck { job_id, status, from, message_id } => {
                                        if let Ok(pid) = PeerId::from_str(&from) {
                                            let key = (job_id.clone(), status.clone(), message_id.clone());
                                            if let Some(p) = pending_job_broadcasts.get_mut(&key) {
                                                p.peers.remove(&pid);
                                                if p.peers.is_empty() {
                                                    pending_job_broadcasts.remove(&key);
                                                }
                                            }
                                        }
                                    }
                                    Command::AnnouncePeers { peers } => {
                                        // Gossip-based peer exchange: dial and add explicit peers
                                        for addr_str in peers.iter() {
                                            if let Ok(ma) = addr_str.parse::<libp2p::Multiaddr>() {
                                                // Extract PeerId if present
                                                if let Some(peer_id) = ma.iter().find_map(|p| {
                                                    if let libp2p::multiaddr::Protocol::P2p(peer_id) = p {
                                                        Some(peer_id)
                                                    } else { None }
                                                }) {
                                                    swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                                                    // Also add to Kademlia DHT routing table
                                                    swarm.behaviour_mut().kademlia.add_address(&peer_id, ma.clone());
                                                }
                                                // Dial the address to establish connection
                                                let _ = swarm.dial(ma);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    SwarmEvent::NewListenAddr { address, .. } => {
                        let dial = format!("{address}/p2p/{local_peer_id}");
                        // Print to stdout so users always see a copy-pastable address
                        println!("Agent listen multiaddr: {dial}");
                        if !ephemeral {
                            // Persist PeerId for CLI whoami/debug
                            let _ = std::fs::create_dir_all(state::agent_data_dir());
                            let _ = std::fs::write(state::agent_data_dir().join("node.peer"), local_peer_id.to_string());
                            // Persist the chosen UDP/TCP port for stable restarts
                            if let Some(port) = address.iter().find_map(|p| match p { libp2p::multiaddr::Protocol::Udp(p) => Some(p), _ => None }) { save_listen_port(port); }
                            if let Some(port) = address.iter().find_map(|p| match p { libp2p::multiaddr::Protocol::Tcp(p) => Some(p), _ => None }) { save_listen_port_tcp(port); }
                        }
                        info!(%dial, "listening");
                    }
                    SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        link_count = link_count.saturating_add(1);
                        // Add new peer to pending job broadcasts
                        for pending in pending_job_broadcasts.values_mut() {
                            pending.peers.insert(peer_id.clone());
                        }
                    }
                    SwarmEvent::ConnectionClosed { .. } => {
                        link_count = link_count.saturating_sub(1);
                    }
                    other => {
                        if cfg!(debug_assertions) { info!(?other, "swarm event"); }
                    }
                }
            }
        }
    }
    
    // This code is unreachable since the main loop runs forever
    // The cleanup will be handled by the graceful shutdown in main.rs
}