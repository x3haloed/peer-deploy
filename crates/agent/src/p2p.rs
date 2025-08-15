use std::time::Duration;
use std::path::PathBuf;
use std::fs;

use base64::Engine;
use anyhow::anyhow;
use futures::StreamExt;
use libp2p::{
    gossipsub, identify, identity, kad, mdns,
    swarm::{Swarm, SwarmEvent},
    Multiaddr, PeerId, SwarmBuilder,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::runner::run_wasm_module_with_limits;
use common::{deserialize_message, serialize_message, Command, REALM_CMD_TOPIC, REALM_STATUS_TOPIC, Status, SignedManifest, verify_bytes_ed25519, Manifest, sha256_hex};

#[derive(libp2p::swarm::NetworkBehaviour)]
struct NodeBehaviour {
    gossipsub: gossipsub::Behaviour,
    kademlia: kad::Behaviour<kad::store::MemoryStore>,
    mdns: mdns::tokio::Behaviour,
    identify: identify::Behaviour,
}

fn load_or_create_node_key() -> identity::Keypair {
    let dir = dirs::data_dir().unwrap_or(std::env::temp_dir()).join("realm-agent");
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

fn agent_data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or(std::env::temp_dir()).join("realm-agent")
}

fn trusted_owner_path() -> PathBuf { agent_data_dir().join("owner.pub") }
fn state_path() -> PathBuf { agent_data_dir().join("state.json") }

fn load_trusted_owner() -> Option<String> {
    fs::read_to_string(trusted_owner_path()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn save_trusted_owner(pub_bs58: &str) {
    let _ = fs::create_dir_all(agent_data_dir());
    let _ = fs::write(trusted_owner_path(), pub_bs58.as_bytes());
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AgentState { last_version: u64 }

fn load_state() -> AgentState {
    if let Ok(bytes) = fs::read(state_path()) {
        if let Ok(s) = serde_json::from_slice::<AgentState>(&bytes) { return s; }
    }
    AgentState::default()
}

fn save_state(state: &AgentState) {
    let _ = fs::create_dir_all(agent_data_dir());
    if let Ok(bytes) = serde_json::to_vec(state) { let _ = fs::write(state_path(), bytes); }
}

pub async fn run_agent(
    wasm_path: Option<String>,
    memory_max_mb: u64,
    fuel: u64,
    epoch_ms: u64,
) -> anyhow::Result<()> {
    let id_keys = load_or_create_node_key();
    let local_peer_id = PeerId::from(id_keys.public());

    let gossip_config = gossipsub::ConfigBuilder::default().build()?;

    let mut gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys.clone()),
        gossip_config,
    ).map_err(|e| anyhow!(e))?;

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
        tokio::spawn(async move {
            let res = run_wasm_module_with_limits(&path, memory_max_mb, fuel, epoch_ms).await
                .map(|_| format!("run ok: {}", path))
                .map_err(|e| format!("run error: {}", e));
            let _ = tx0.send(res);
        });
    }

    info!(peer = %local_peer_id, "agent started");

    let mut interval = tokio::time::interval(Duration::from_secs(5));

    loop {
        tokio::select! {
            Some(run_res) = rx.recv() => {
                let msg = match run_res { Ok(m) => m, Err(m) => m };
                let status = Status { node_id: local_peer_id.to_string(), msg };
                let _ = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), serialize_message(&status));
            }
            _ = interval.tick() => {
                let status = Status { node_id: local_peer_id.to_string(), msg: "alive".into() };
                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), serialize_message(&status)) {
                    warn!(error=%e, "failed to publish heartbeat status");
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
                                match cmd {
                                    Command::Hello { from } => {
                                        let msg = Status { node_id: local_peer_id.to_string(), msg: format!("hello, {}", from) };
                                        let _ = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), serialize_message(&msg));
                                    }
                                    Command::Run { wasm_path, memory_max_mb, fuel, epoch_ms } => {
                                        let tx1 = tx.clone();
                                        tokio::spawn(async move {
                                            let res = run_wasm_module_with_limits(&wasm_path, memory_max_mb, fuel, epoch_ms).await
                                                .map(|_| format!("run ok: {}", wasm_path))
                                                .map_err(|e| format!("run error: {}", e));
                                            let _ = tx1.send(res);
                                        });
                                    }
                                    Command::ApplyManifest(signed) => {
                                        let tx2 = tx.clone();
                                        tokio::spawn(async move {
                                            handle_apply_manifest(tx2, signed).await;
                                        });
                                    }
                                    Command::StatusQuery => {
                                        let msg = Status { node_id: local_peer_id.to_string(), msg: "ok".into() };
                                        let _ = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), serialize_message(&msg));
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

async fn handle_apply_manifest(
    tx: tokio::sync::mpsc::UnboundedSender<Result<String, String>>,
    signed: SignedManifest,
) {
    // Signature check
    let sig = match base64::engine::general_purpose::STANDARD.decode(&signed.signature_b64) {
        Ok(s) => s,
        Err(e) => { let _ = tx.send(Err(format!("bad signature_b64: {}", e))); return; }
    };
    let ok = verify_bytes_ed25519(&signed.owner_pub_bs58, signed.manifest_toml.as_bytes(), &sig)
        .unwrap_or(false);
    if !ok { let _ = tx.send(Err("manifest rejected (sig)".into())); return; }
    // TOFU
    if let Some(trusted) = load_trusted_owner() {
        if trusted != signed.owner_pub_bs58 { let _ = tx.send(Err("manifest rejected (owner mismatch)".into())); return; }
    } else { save_trusted_owner(&signed.owner_pub_bs58); }
    // Monotonic version
    let state = load_state();
    if state.last_version >= signed.version {
        let _ = tx.send(Err(format!("manifest rejected (stale v{} <= v{})", signed.version, state.last_version)));
        return;
    }
    // Verify and stage artifacts, then launch and persist version
    match verify_and_stage_artifacts(&signed.manifest_toml).await {
        Ok(staged) => {
            if let Err(e) = launch_components(staged, &signed.manifest_toml).await {
                let _ = tx.send(Err(format!("launch error: {}", e))); return;
            }
            let mut state2 = load_state();
            state2.last_version = signed.version;
            save_state(&state2);
            let _ = tx.send(Ok(format!("manifest accepted v{}", signed.version)));
        }
        Err(e) => { let _ = tx.send(Err(format!("manifest rejected (digest): {}", e))); }
    }
}

async fn fetch_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    if let Some(rest) = url.strip_prefix("file:") {
        let path = std::path::Path::new(rest);
        return Ok(tokio::fs::read(path).await?);
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        let res = reqwest::get(url).await?;
        let status = res.status();
        if !status.is_success() {
            return Err(anyhow::anyhow!("fetch {}: {}", url, status));
        }
        let bytes = res.bytes().await?;
        return Ok(bytes.to_vec());
    }
    Err(anyhow::anyhow!("unsupported source: {}", url))
}

async fn verify_and_stage_artifacts(manifest_toml: &str) -> anyhow::Result<std::collections::BTreeMap<String, std::path::PathBuf>> {
    let manifest: Manifest = toml::from_str(manifest_toml)?;
    let mut staged = std::collections::BTreeMap::new();
    let stage_dir = agent_data_dir().join("artifacts");
    tokio::fs::create_dir_all(&stage_dir).await.ok();
    for (name, comp) in manifest.components.iter() {
        let bytes = fetch_bytes(&comp.source).await?;
        let digest = sha256_hex(&bytes);
        if digest != comp.sha256_hex {
            return Err(anyhow::anyhow!("component {} digest mismatch", name));
        }
        let file_path = stage_dir.join(format!("{}-{}.wasm", name, &digest[..16]));
        if !file_path.exists() {
            tokio::fs::write(&file_path, &bytes).await?;
        }
        staged.insert(name.clone(), file_path);
    }
    Ok(staged)
}

async fn launch_components(staged: std::collections::BTreeMap<String, std::path::PathBuf>, manifest_toml: &str) -> anyhow::Result<()> {
    let manifest: Manifest = toml::from_str(manifest_toml)?;
    for (name, path) in staged {
        if let Some(spec) = manifest.components.get(&name) {
            let mem = spec.memory_max_mb.unwrap_or(64);
            let fuel = spec.fuel.unwrap_or(5_000_000);
            let epoch = spec.epoch_ms.unwrap_or(100);
            let p = path.to_string_lossy().to_string();
            tokio::spawn(async move {
                if let Err(e) = run_wasm_module_with_limits(&p, mem, fuel, epoch).await {
                    warn!(component=%name, error=%e, "component run failed");
                }
            });
        }
    }
    Ok(())
}
