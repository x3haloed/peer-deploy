#![allow(clippy::collapsible_match)]

use std::time::{Duration, Instant};

use futures::StreamExt;

use common::{serialize_message, Command};

use super::util::{mdns_warmup, new_swarm, NodeBehaviourEvent, dial_bootstrap};

pub async fn status() -> anyhow::Result<()> {
    let (mut swarm, topic_cmd, topic_status) = new_swarm().await?;
    libp2p::Swarm::listen_on(
        &mut swarm,
        "/ip4/0.0.0.0/udp/0/quic-v1"
            .parse::<libp2p::Multiaddr>()
            .map_err(|e| anyhow::anyhow!("Failed to parse multiaddr: {}", e))?,
    )?;

    // Warm up discovery and dial bootstrap before publishing to avoid InsufficientPeers
    mdns_warmup(&mut swarm).await;
    dial_bootstrap(&mut swarm).await;

    // Retry publish until we receive a reply or the deadline is reached
    let deadline = Instant::now() + Duration::from_secs(7);
    let mut republish = tokio::time::interval(Duration::from_millis(800));
    // initial publish
    let _ = swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic_cmd.clone(), serialize_message(&Command::StatusQuery));

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline.into()) => {
                println!("no status received");
                return Ok(());
            }
            _ = republish.tick() => {
                let _ = swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(topic_cmd.clone(), serialize_message(&Command::StatusQuery));
            }
            event = swarm.select_next_some() => {
                match event {
                    libp2p::swarm::SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(ev)) => match ev {
                        libp2p::mdns::Event::Discovered(list) => {
                            for (peer, _addr) in list {
                                swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer);
                            }
                        }
                        libp2p::mdns::Event::Expired(list) => {
                            for (peer, _addr) in list {
                                swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer);
                            }
                        }
                    },
                    libp2p::swarm::SwarmEvent::Behaviour(NodeBehaviourEvent::Gossipsub(ev)) => {
                        if let libp2p::gossipsub::Event::Message { message, .. } = ev {
                            if message.topic == topic_status.hash() {
                                println!("{}", String::from_utf8_lossy(&message.data));
                                return Ok(());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
