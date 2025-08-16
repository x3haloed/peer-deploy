use std::time::Duration;

use futures::StreamExt;

use common::{serialize_message, Command};

use super::util::{new_swarm, mdns_warmup, NodeBehaviourEvent};

pub async fn status() -> anyhow::Result<()> {
    let (mut swarm, topic_cmd, topic_status) = new_swarm().await?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/udp/0/quic-v1".parse::<libp2p::Multiaddr>().unwrap())?;

    swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic_cmd.clone(), serialize_message(&Command::StatusQuery))?;

    let timeout = tokio::time::timeout(Duration::from_secs(5), async {
        mdns_warmup(&mut swarm).await;
        loop {
            match swarm.select_next_some().await {
                libp2p::swarm::SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(ev)) => {
                    match ev {
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
                    }
                }
                libp2p::swarm::SwarmEvent::Behaviour(NodeBehaviourEvent::Gossipsub(ev)) => {
                    if let libp2p::gossipsub::Event::Message { message, .. } = ev {
                        if message.topic == topic_status.hash() {
                            println!("{}", String::from_utf8_lossy(&message.data));
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
    });

    match timeout.await {
        Ok(_) => Ok(()),
        Err(_) => {
            println!("no status received");
            Ok(())
        }
    }
}

