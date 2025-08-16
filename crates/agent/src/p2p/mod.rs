#![allow(clippy::collapsible_match, clippy::double_ended_iterator_last)]

use std::sync::{atomic::Ordering, Arc};
use std::time::Duration;

use anyhow::anyhow;
use futures::StreamExt;
use libp2p::{
    gossipsub, identify, identity, kad, mdns,
    swarm::{Swarm, SwarmEvent},
    Multiaddr, PeerId, SwarmBuilder,
};
use tracing::{info, warn};

use crate::runner::run_wasm_module_with_limits;
use common::{
    deserialize_message, serialize_message, Command, Status, REALM_CMD_TOPIC, REALM_STATUS_TOPIC,
};

mod handlers;
pub mod metrics;
mod state;

use handlers::{handle_apply_manifest, handle_upgrade};
use metrics::{push_log, serve_metrics, Metrics, SharedLogs};
use state::load_state;

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

pub async fn run_agent(
    wasm_path: Option<String>,
    memory_max_mb: u64,
    fuel: u64,
    epoch_ms: u64,
) -> anyhow::Result<()> {
    let metrics = Arc::new(Metrics::new());
    let logs: SharedLogs =
        std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::BTreeMap::new()));
    let sys = std::sync::Arc::new(tokio::sync::Mutex::new(sysinfo::System::new_all()));

    let id_keys = load_or_create_node_key();
    let local_peer_id = PeerId::from(id_keys.public());

    let gossip_config = gossipsub::ConfigBuilder::default().build()?;

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

    let identify = identify::Behaviour::new(identify::Config::new(
        "peer-deploy/0.1".into(),
        id_keys.public(),
    ));

    let behaviour = NodeBehaviour {
        gossipsub,
        kademlia,
        mdns,
        identify,
    };

    let mut swarm = SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_quic()
        .with_dns()?
        .with_behaviour(|_| Ok(behaviour))?
        .build();

    let listen_addr: Multiaddr = "/ip4/0.0.0.0/udp/0/quic-v1".parse().unwrap();
    Swarm::listen_on(&mut swarm, listen_addr)?;

    // channel for run results to publish status from the main loop
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Result<String, String>>();

    if let Some(path) = wasm_path.clone() {
        let tx0 = tx.clone();
        let logs0 = logs.clone();
        tokio::spawn(async move {
            push_log(&logs0, "adhoc", format!("starting run {path}")).await;
            let res = run_wasm_module_with_limits(
                &path,
                "adhoc",
                logs0.clone(),
                memory_max_mb,
                fuel,
                epoch_ms,
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

    info!(peer = %local_peer_id, "agent started");

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

    let mut interval = tokio::time::interval(Duration::from_secs(5));

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
                    tags: vec![],
                };
                if let Err(_e) = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), serialize_message(&status)) {
                    metrics.status_publish_errors_total.fetch_add(1, Ordering::Relaxed);
                } else {
                    metrics.status_published_total.fetch_add(1, Ordering::Relaxed);
                }
            }
            _ = interval.tick() => {
                let (cpu_percent, mem_percent) = {
                    let mut s = sys.lock().await;
                    s.refresh_all();
                    let cpu = (s.global_cpu_info().cpu_usage() as u64).min(100);
                    let mem = if s.total_memory() == 0 { 0 } else { ((s.used_memory() as f64 / s.total_memory() as f64) * 100.0) as u64 };
                    (cpu, mem)
                };
                let status = Status {
                    node_id: local_peer_id.to_string(),
                    msg: "alive".into(),
                    agent_version: metrics.agent_version.load(Ordering::Relaxed),
                    components_desired: metrics.components_desired.load(Ordering::Relaxed),
                    components_running: metrics.components_running.load(Ordering::Relaxed),
                    cpu_percent,
                    mem_percent,
                    tags: vec![],
                };
                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), serialize_message(&status)) {
                    warn!(error=%e, "failed to publish heartbeat status");
                    metrics.status_publish_errors_total.fetch_add(1, Ordering::Relaxed);
                } else {
                    metrics.status_published_total.fetch_add(1, Ordering::Relaxed);
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
                                            tags: vec![],
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
                                        tokio::spawn(async move {
                                            push_log(&logs1, "adhoc", format!("starting run {wasm_path}")).await;
                                            let res = run_wasm_module_with_limits(&wasm_path, "adhoc", logs1.clone(), memory_max_mb, fuel, epoch_ms).await
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
                                        tokio::spawn(async move {
                                            push_log(&logs2, "apply", format!("apply v{}", signed.version)).await;
                                            handle_apply_manifest(tx2, signed, logs2, m2).await;
                                        });
                                    }
                                    Command::UpgradeAgent(pkg) => {
                                        let tx3 = tx.clone();
                                        let logs3 = logs.clone();
                                        tokio::spawn(async move {
                                            let _ = logs3; // placeholder to wire logs into upgrade path later
                                            handle_upgrade(tx3, pkg).await;
                                        });
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
                                            tags: vec![],
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
                                }
                            }
                        }
                    }
                    SwarmEvent::NewListenAddr { address, .. } => {
                        info!(%address, "listening");
                    }
                    other => {
                        if cfg!(debug_assertions) { info!(?other, "swarm event"); }
                    }
                }
            }
        }
    }
}
