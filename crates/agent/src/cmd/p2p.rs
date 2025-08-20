use crate::cmd::util::{new_swarm, mdns_warmup, dial_bootstrap, NodeBehaviourEvent};
use futures::StreamExt;
use libp2p::swarm::SwarmEvent;
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::Result;

/// Watch all P2P messages in real time on the CLI
pub async fn watch() -> Result<()> {
    let (mut swarm, cmd_topic, status_topic) = new_swarm().await?;
    println!("Starting P2P watch with rate limiting. Press Ctrl+C to stop.\n");
    mdns_warmup(&mut swarm).await;
    dial_bootstrap(&mut swarm).await;

    let mut message_count = 0u64;
    let mut last_report = std::time::Instant::now();
    let mut rate_limiter = std::time::Instant::now();
    
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\nP2P watch terminated by user");
                println!("Total messages received: {}", message_count);
                break;
            }
            event = swarm.select_next_some() => {
                if let SwarmEvent::Behaviour(NodeBehaviourEvent::Gossipsub(ev)) = event {
                    if let libp2p::gossipsub::Event::Message { propagation_source, message, .. } = ev {
                        message_count += 1;
                        
                        // Rate limit console output to max 5 messages per second
                        let now = std::time::Instant::now();
                        if now.duration_since(rate_limiter).as_millis() >= 200 {
                            rate_limiter = now;
                            
                            let topic_type = if message.topic == cmd_topic.hash() {
                                "CMD"
                            } else if message.topic == status_topic.hash() {
                                "STATUS"
                            } else {
                                "UNKNOWN"
                            };
                            
                            let ts = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or_default();
                            let payload = String::from_utf8_lossy(&message.data);
                            let preview = if payload.len() > 200 { 
                                format!("{}...", &payload[..200])
                            } else {
                                payload.to_string()
                            };
                            println!("[{}][{}] from {} (#{} total): {}", ts, topic_type, propagation_source, message_count, preview);
                        }
                        
                        // Report message rate every 5 seconds
                        if now.duration_since(last_report).as_secs() >= 5 {
                            let elapsed = now.duration_since(last_report).as_secs_f64();
                            let rate = message_count as f64 / elapsed;
                            println!(">>> MESSAGE RATE: {:.1} messages/second (total: {})", rate, message_count);
                            last_report = now;
                            message_count = 0; // Reset for next interval
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
