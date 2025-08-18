use anyhow::Context;
use futures::StreamExt;
use libp2p::{gossipsub, identity, mdns, noise, yamux, tcp, swarm::SwarmEvent, Multiaddr, SwarmBuilder};

use crate::cmd::util::owner_dir;
use common::{REALM_CMD_TOPIC, REALM_STATUS_TOPIC};

#[derive(libp2p::swarm::NetworkBehaviour)]
pub struct NodeBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
}

pub async fn diag_quic(addr: String) -> anyhow::Result<()> {
    let id_keys = identity::Keypair::generate_ed25519();
    let gossip_config = gossipsub::ConfigBuilder::default().build()?;
    let gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys.clone()),
        gossip_config,
    )
    .map_err(|e| anyhow::anyhow!(e))?;
    let mdns_beh = mdns::tokio::Behaviour::new(mdns::Config::default(), libp2p::PeerId::from(id_keys.public()))?;
    let behaviour = NodeBehaviour { gossipsub, mdns: mdns_beh };
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

    println!("diag: submitting dial {addr}");
    let ma: Multiaddr = addr.parse().context("parse multiaddr")?;
    if let Err(e) = libp2p::Swarm::dial(&mut swarm, ma.clone()) {
        println!("diag: dial submit error: {e}");
        return Ok(());
    }

    let start = std::time::Instant::now();
    let mut got_event = false;
    while start.elapsed() < std::time::Duration::from_secs(5) {
        if let Some(ev) = swarm.next().await {
            got_event = true;
            match ev {
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    println!("diag: connection established with {peer_id}");
                    break;
                }
                SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    println!("diag: outgoing connection error to {:?}: {}", peer_id, error);
                    break;
                }
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("diag: listener {address}");
                }
                other => {
                    // verbose surface for debugging
                    println!("diag: swarm event: {:?}", other); 
                }
            }
        }
    }
    if !got_event {
        println!("diag: no swarm events within 5s (likely no UDP packets or blocked QUIC)");
    }
    Ok(())
}


