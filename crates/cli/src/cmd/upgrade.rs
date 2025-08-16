use std::time::Duration;

use anyhow::Context;
use base64::Engine;

use common::{serialize_message, Command, OwnerKeypair, AgentUpgrade, sign_bytes_ed25519, sha256_hex};

use super::util::{new_swarm, mdns_warmup, owner_dir};

pub async fn upgrade(
    file: String,
    version: u64,
    target_peers: Vec<String>,
    target_tags: Vec<String>,
) -> anyhow::Result<()> {
    let (mut swarm, topic_cmd, _topic_status) = new_swarm().await?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/udp/0/quic-v1".parse::<libp2p::Multiaddr>().unwrap())?;

    mdns_warmup(&mut swarm).await;

    // load owner key
    let dir = owner_dir()?;
    let key_path = dir.join("owner.key.json");
    let bytes = tokio::fs::read(&key_path).await.context("read owner key")?;
    let kp: OwnerKeypair = serde_json::from_slice(&bytes)?;

    let bin_bytes = tokio::fs::read(&file).await?;
    let digest = sha256_hex(&bin_bytes);
    let sig = sign_bytes_ed25519(&kp.private_hex, &bin_bytes)?;
    let pkg = AgentUpgrade {
        alg: "ed25519".into(),
        owner_pub_bs58: kp.public_bs58.clone(),
        version,
        target_peer_ids: target_peers,
        target_tags,
        binary_sha256_hex: digest,
        binary_b64: base64::engine::general_purpose::STANDARD.encode(&bin_bytes),
        signature_b64: base64::engine::general_purpose::STANDARD.encode(sig),
    };
    swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic_cmd.clone(), serialize_message(&Command::UpgradeAgent(pkg)))?;

    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(())
}

