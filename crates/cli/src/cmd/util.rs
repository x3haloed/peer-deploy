#![allow(clippy::collapsible_match)]

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Context};
use futures::StreamExt;
use libp2p::{
    gossipsub, mdns,
    swarm::{Swarm, SwarmEvent},
    PeerId, SwarmBuilder,
};

use common::{REALM_CMD_TOPIC, REALM_STATUS_TOPIC};

/// Directory where the owner's key material is stored.
pub fn owner_dir() -> anyhow::Result<PathBuf> {
    Ok(dirs::config_dir().context("config dir")?.join("realm"))
}

/// Directory used by the agent for state files.
pub fn agent_data_dir_cli() -> anyhow::Result<PathBuf> {
    Ok(dirs::data_dir().context("data dir")?.join("realm-agent"))
}

pub async fn write_trusted_owner(pub_bs58: &str) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(&agent_data_dir_cli()?).await?;
    let path = agent_data_dir_cli()?.join("owner.pub");
    tokio::fs::write(path, pub_bs58.as_bytes()).await?;
    Ok(())
}

pub async fn write_bootstrap(addrs: &[String]) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(&agent_data_dir_cli()?).await?;
    let path = agent_data_dir_cli()?.join("bootstrap.json");
    let bytes = serde_json::to_vec_pretty(&addrs)?;
    tokio::fs::write(path, bytes).await?;
    Ok(())
}

#[derive(libp2p::swarm::NetworkBehaviour)]
pub struct NodeBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
}

/// Create a new swarm suitable for CLI interactions.
pub async fn new_swarm() -> anyhow::Result<(
    Swarm<NodeBehaviour>,
    gossipsub::IdentTopic,
    gossipsub::IdentTopic,
)> {
    let id_keys = libp2p::identity::Keypair::generate_ed25519();

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

    let mdns_beh =
        mdns::tokio::Behaviour::new(mdns::Config::default(), PeerId::from(id_keys.public()))?;

    let behaviour = NodeBehaviour {
        gossipsub,
        mdns: mdns_beh,
    };

    let swarm = SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_quic()
        .with_dns()?
        .with_behaviour(|_| Ok(behaviour))?
        .build();

    Ok((swarm, topic_cmd, topic_status))
}

/// Briefly process mDNS events to warm up peer discovery.
pub async fn mdns_warmup(swarm: &mut Swarm<NodeBehaviour>) {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(600) {
        if let Some(event) = swarm.next().await {
            if let SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(ev)) = event {
                match ev {
                    mdns::Event::Discovered(list) => {
                        for (peer, _addr) in list {
                            swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer);
                        }
                    }
                    mdns::Event::Expired(list) => {
                        for (peer, _addr) in list {
                            swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer);
                        }
                    }
                }
            }
        }
    }
}
