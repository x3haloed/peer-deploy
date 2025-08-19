use anyhow::Context;
use common::{serialize_message, Command, JobSpec, OwnerKeypair};

use super::util::{new_swarm, mdns_warmup, owner_dir};

pub async fn submit_job(job_toml_path: String) -> anyhow::Result<()> {
    let (mut swarm, topic_cmd, _topic_status) = new_swarm().await?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/udp/0/quic-v1".parse::<libp2p::Multiaddr>()
        .map_err(|e| anyhow::anyhow!("Failed to parse multiaddr: {}", e))?)?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/tcp/0".parse::<libp2p::Multiaddr>()
        .map_err(|e| anyhow::anyhow!("Failed to parse multiaddr: {}", e))?)?;

    mdns_warmup(&mut swarm).await;

    // load owner key to ensure presence (no signing yet for job spec; TODO: signed jobs)
    let dir = owner_dir()?;
    let key_path = dir.join("owner.key.json");
    let _kp_bytes = tokio::fs::read(&key_path).await.context("read owner key")?;
    let _kp: OwnerKeypair = serde_json::from_slice(&_kp_bytes)?;

    let text = tokio::fs::read_to_string(&job_toml_path).await?;
    let spec: JobSpec = toml::from_str(&text)?;

    let msg = Command::SubmitJob(spec);
    swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic_cmd.clone(), serialize_message(&msg))?;
    Ok(())
}


