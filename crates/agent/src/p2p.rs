use std::time::Duration;

use anyhow::anyhow;
use futures::StreamExt;
use libp2p::{
    gossipsub, identify, identity, kad, mdns,
    swarm::{Swarm, SwarmEvent},
    Multiaddr, PeerId, SwarmBuilder,
};
use tracing::{error, info, warn};

use crate::runner::run_wasm_module_with_limits;
use common::{deserialize_message, serialize_message, Command, REALM_CMD_TOPIC, REALM_STATUS_TOPIC, Status};

#[derive(libp2p::swarm::NetworkBehaviour)]
struct NodeBehaviour {
    gossipsub: gossipsub::Behaviour,
    kademlia: kad::Behaviour<kad::store::MemoryStore>,
    mdns: mdns::tokio::Behaviour,
    identify: identify::Behaviour,
}

pub async fn run_agent(
    wasm_path: Option<String>,
    memory_max_mb: u64,
    fuel: u64,
    epoch_ms: u64,
) -> anyhow::Result<()> {
    let id_keys = identity::Keypair::generate_ed25519();
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

    if let Some(path) = wasm_path.clone() {
        tokio::spawn(async move {
            match run_wasm_module_with_limits(&path, memory_max_mb, fuel, epoch_ms).await {
                Ok(_) => info!("wasm module finished successfully"),
                Err(e) => error!(error = %e, "wasm module failed"),
            }
        });
    }

    info!(peer = %local_peer_id, "agent started");

    let mut interval = tokio::time::interval(Duration::from_secs(5));

    loop {
        tokio::select! {
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
                                info!(from=%propagation_source, ?message_id, ?cmd, "received command");
                                match cmd {
                                    Command::Hello { from } => {
                                        let msg = Status { node_id: local_peer_id.to_string(), msg: format!("hello, {}", from) };
                                        let _ = swarm.behaviour_mut().gossipsub.publish(topic_status.clone(), serialize_message(&msg));
                                    }
                                    Command::Run { wasm_path, memory_max_mb, fuel, epoch_ms } => {
                                        let _ = tokio::spawn(async move {
                                            if let Err(e) = run_wasm_module_with_limits(&wasm_path, memory_max_mb, fuel, epoch_ms).await {
                                                error!(error=%e, path=%wasm_path, "run command failed");
                                            }
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
