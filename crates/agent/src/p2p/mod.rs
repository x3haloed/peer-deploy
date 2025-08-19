#![allow(clippy::collapsible_match, clippy::double_ended_iterator_last)]

use std::sync::{atomic::Ordering, Arc};
use std::time::Duration;

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
    load_bootstrap_addrs, load_state, load_trusted_owner,
    load_listen_port, save_listen_port,
    load_listen_port_tcp, save_listen_port_tcp,
    update_persistent_manifest_with_component
};

mod handlers;
mod jobs;
mod jobs_wasm;
mod jobs_native;
mod jobs_qemu;
pub mod metrics;
pub mod state;  // Make state module public
mod gateway;

use handlers::{handle_apply_manifest, handle_upgrade};
use jobs::{execute_oneshot_job, execute_service_job};
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
    
    supervisor.clone().spawn_reconcile();

    // Initialize job manager and restore job state
    let job_manager = std::sync::Arc::new(crate::job_manager::JobManager::new(
        state::agent_data_dir().join("jobs")
    ));
    
    if let Err(e) = job_manager.load_from_disk().await {
        warn!(error=%e, "Failed to restore job state from disk, starting fresh");
    }

    let id_keys = load_or_create_node_key();
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

    // Use a persisted UDP port if available to keep stable multiaddrs across restarts
    let listen_addr: Multiaddr = if let Some(port) = load_listen_port() {
        format!("/ip4/0.0.0.0/udp/{}/quic-v1", port).parse()
            .map_err(|e| anyhow!("Failed to parse UDP multiaddr: {}", e))?
    } else {
        "/ip4/0.0.0.0/udp/0/quic-v1".parse()
            .map_err(|e| anyhow!("Failed to parse UDP multiaddr: {}", e))?
    };
    Swarm::listen_on(&mut swarm, listen_addr)?;
    // Also listen on TCP to support environments where UDP/QUIC is unavailable; reuse persisted port if any
    let listen_tcp: Multiaddr = if let Some(port) = load_listen_port_tcp() {
        format!("/ip4/0.0.0.0/tcp/{}", port).parse()
            .map_err(|e| anyhow!("Failed to parse TCP multiaddr: {}", e))?
    } else {
        "/ip4/0.0.0.0/tcp/0".parse()
            .map_err(|e| anyhow!("Failed to parse TCP multiaddr: {}", e))?
    };
    Swarm::listen_on(&mut swarm, listen_tcp)?;
    // Dial configured bootstrap peers
    for addr in load_bootstrap_addrs().into_iter() {
        if let Ok(ma) = addr.parse::<Multiaddr>() {
            let _ = libp2p::Swarm::dial(&mut swarm, ma);
        }
    }

    // Track current number of established connections to report in Status
    let mut link_count: usize = 0;

    // channel for run results to publish status from the main loop
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Result<String, String>>();

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

    let mut interval = tokio::time::interval(Duration::from_secs(5));
    let mut storage_announce_tick = tokio::time::interval(Duration::from_secs(60));
    let mut schedule_tick = tokio::time::interval(Duration::from_secs(60));
    // track msgs per second by counting status publishes
    let mut last_publish_count: u64 = 0;
    let mut last_sample_time = std::time::Instant::now();

    loop {
        tokio::select! {
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
                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), serialize_message(&status)) {
                    warn!(error=%e, "failed to publish heartbeat status");
                    metrics.status_publish_errors_total.fetch_add(1, Ordering::Relaxed);
                } else {
                    metrics.status_published_total.fetch_add(1, Ordering::Relaxed);
                    last_publish_count = last_publish_count.saturating_add(1);
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
            _ = schedule_tick.tick() => {
                // Evaluate recurring job schedules
                if let Ok(due_specs) = job_manager.evaluate_schedules().await {
                    for spec in due_specs {
                        // Re-publish the SubmitJob command so eligible nodes can take it
                        let _ = swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), serialize_message(&common::Command::SubmitJob(spec)));
                    }
                }
            }
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(ev)) => {
                        match ev {
                            mdns::Event::Discovered(list) => {
                                for (peer, _addr) in list { swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer); }
                            }
                            mdns::Event::Expired(list) => {
                                for (peer, _addr) in list { swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer); }
                            }
                        }
                    }
                    SwarmEvent::Behaviour(NodeBehaviourEvent::Gossipsub(ev)) => {
                        if let gossipsub::Event::Message { propagation_source, message_id, message } = ev {
                            if let Ok(cmd) = deserialize_message::<Command>(&message.data) {
                                info!(from=%propagation_source, ?message_id, "received command");
                                metrics.commands_received_total.fetch_add(1, Ordering::Relaxed);
                                match cmd {
                                    common::Command::StorageHave { digest, size: _ } => {
                                        // Track which peers have which content (minimal for now)
                                        // Future: maintain a map digest -> set of peers for locality
                                        // For now, just update peer status to show activity
                                        let s = Status {
                                            node_id: local_peer_id.to_string(),
                                            msg: format!("have:{}", &digest[..std::cmp::min(12, digest.len())]),
                                            agent_version: metrics.agent_version.load(Ordering::Relaxed),
                                            components_desired: metrics.components_desired.load(Ordering::Relaxed),
                                            components_running: metrics.components_running.load(Ordering::Relaxed),
                                            cpu_percent: 0,
                                            mem_percent: 0,
                                            tags: roles.clone(),
                                            drift: 0,
                                            trusted_owner_pub_bs58: load_trusted_owner(),
                                            links: link_count as u64,
                                        };
                                        let _ = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), serialize_message(&s));
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
                                        tokio::spawn(async move {
                                            // Create job instance and track it
                                            let job_id = match job_mgr.submit_job(job.clone()).await {
                                                Ok(id) => id,
                                                Err(e) => {
                                                    let _ = txj.send(Err(format!("job submission failed: {}", e)));
                                                    return;
                                                }
                                            };
                                            
                                            // basic targeting: platform + tags + node_ids
                                            let mut selected = true;
                                            if let Some(t) = &job.targeting {
                                                if let Some(p) = &t.platform {
                                                    let host = format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH);
                                                    if &host != p { selected = false; }
                                                }
                                                if selected && !t.tags.is_empty() {
                                                    selected = t.tags.iter().any(|tag| rolesj.contains(tag));
                                                }
                                            }
                                            if selected {
                                                // Mark job as started
                                                let _ = job_mgr.start_job(&job_id, node_id).await;
                                                let _ = job_mgr.add_job_log(&job_id, "info".to_string(), "Job execution started on this node".to_string()).await;
                                                
                                                // Handle different job types
                                                match &job.job_type {
                                                    common::JobType::OneShot => {
                                                        execute_oneshot_job(job_mgr.clone(), job_id.clone(), job.clone(), logsj.clone(), txj.clone()).await;
                                                    },
                                                    common::JobType::Recurring => {
                                                        // Recurring jobs are handled by the scheduler, treat execution as one-shot
                                                        execute_oneshot_job(job_mgr.clone(), job_id.clone(), job.clone(), logsj.clone(), txj.clone()).await;
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
                                                            execute_service_job(service_job_mgr, service_job_id, service_job, service_logs, service_tx, cancel_rx).await;
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
                                }
                            }
                        }
                    }
                    SwarmEvent::NewListenAddr { address, .. } => {
                        let dial = format!("{address}/p2p/{local_peer_id}");
                        // Print to stdout so users always see a copy-pastable address
                        println!("Agent listen multiaddr: {dial}");
                        // Persist PeerId for CLI whoami/debug
                        let _ = std::fs::create_dir_all(state::agent_data_dir());
                        let _ = std::fs::write(state::agent_data_dir().join("node.peer"), local_peer_id.to_string());
                        // Persist the chosen UDP/TCP port for stable restarts
                        if let Some(port) = address.iter().find_map(|p| match p { libp2p::multiaddr::Protocol::Udp(p) => Some(p), _ => None }) { save_listen_port(port); }
                        if let Some(port) = address.iter().find_map(|p| match p { libp2p::multiaddr::Protocol::Tcp(p) => Some(p), _ => None }) { save_listen_port_tcp(port); }
                        info!(%dial, "listening");
                    }
                    SwarmEvent::ConnectionEstablished { .. } => {
                        link_count = link_count.saturating_add(1);
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