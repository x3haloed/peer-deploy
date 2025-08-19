use crate::cmd::util::{new_swarm, mdns_warmup, dial_bootstrap, NodeBehaviourEvent};
use futures::StreamExt;
use libp2p::swarm::SwarmEvent;
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::Result;

/// Watch all P2P messages in real time on the CLI
pub async fn watch() -> Result<()> {
    let (mut swarm, cmd_topic, status_topic) = new_swarm().await?;
    println!("Starting P2P watch. Press Ctrl+C to stop.\n");
    mdns_warmup(&mut swarm).await;
    dial_bootstrap(&mut swarm).await;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\nP2P watch terminated by user");
                break;
            }
            event = swarm.select_next_some() => {
                if let SwarmEvent::Behaviour(NodeBehaviourEvent::Gossipsub(ev)) = event {
                    if let libp2p::gossipsub::Event::Message { propagation_source, message, .. } = ev {
                        // Determine topic type
                        let topic_type = if message.topic == cmd_topic.hash() {
                            "CMD"
                        } else if message.topic == status_topic.hash() {
                            "STATUS"
                        } else {
                            "UNKNOWN"
                        };
                        // Timestamp in seconds since epoch
                        let ts = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or_default();
                        let payload = String::from_utf8_lossy(&message.data);
                        println!("[{}][{}] from {}: {}", ts, topic_type, propagation_source, payload);
                    }
                }
            }
        }
    }
    Ok(())
}
