use libp2p::{gossipsub, identity, mdns, ping, PeerId, SwarmBuilder};

use common::{REALM_CMD_TOPIC, REALM_STATUS_TOPIC};

#[derive(libp2p::swarm::NetworkBehaviour)]
pub struct NodeBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
    pub ping: ping::Behaviour,
}

pub async fn new_swarm_tui() -> anyhow::Result<(
    libp2p::Swarm<NodeBehaviour>,
    gossipsub::IdentTopic,
    gossipsub::IdentTopic,
    PeerId,
)> {
    let id_keys = identity::Keypair::generate_ed25519();
    let gossip_config = gossipsub::ConfigBuilder::default().build()?;
    let mut gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys.clone()),
        gossip_config,
    )
    .map_err(|e| anyhow::anyhow!(e))?;
    let topic_cmd = gossipsub::IdentTopic::new(REALM_CMD_TOPIC);
    let topic_status = gossipsub::IdentTopic::new(REALM_STATUS_TOPIC);
    gossipsub.subscribe(&topic_cmd)?;
    gossipsub.subscribe(&topic_status)?;
    let mdns_beh =
        mdns::tokio::Behaviour::new(mdns::Config::default(), PeerId::from(id_keys.public()))?;
    let ping_beh = ping::Behaviour::new(ping::Config::new());
    let behaviour = NodeBehaviour {
        gossipsub,
        mdns: mdns_beh,
        ping: ping_beh,
    };
    let local_peer_id = PeerId::from(id_keys.public());
    let swarm = SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_quic()
        .with_dns()?
        .with_behaviour(|_| Ok(behaviour))?
        .build();
    Ok((swarm, topic_cmd, topic_status, local_peer_id))
}
