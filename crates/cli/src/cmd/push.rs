use anyhow::Context;
use base64::Engine;

use common::{sha256_hex, sign_bytes_ed25519, serialize_message, Command, OwnerKeypair, PushPackage, PushUnsigned};

use super::util::{mdns_warmup, new_swarm, owner_dir};

pub async fn push(
    name: String,
    file: String,
    replicas: u32,
    memory_max_mb: u64,
    fuel: u64,
    epoch_ms: u64,
    target_peers: Vec<String>,
    target_tags: Vec<String>,
    start: bool,
) -> anyhow::Result<()> {
    let (mut swarm, topic_cmd, _topic_status) = new_swarm().await?;
    libp2p::Swarm::listen_on(
        &mut swarm,
        "/ip4/0.0.0.0/udp/0/quic-v1".parse::<libp2p::Multiaddr>().unwrap(),
    )?;

    mdns_warmup(&mut swarm).await;

    // load owner key
    let dir = owner_dir()?;
    let key_path = dir.join("owner.key.json");
    let bytes = tokio::fs::read(&key_path).await.context("read owner key")?;
    let kp: OwnerKeypair = serde_json::from_slice(&bytes)?;

    let bin = tokio::fs::read(&file).await.context("read wasm")?;
    let digest = sha256_hex(&bin);

    let unsigned = PushUnsigned {
        alg: "ed25519".into(),
        owner_pub_bs58: kp.public_bs58.clone(),
        component_name: name,
        target_peer_ids: target_peers,
        target_tags,
        memory_max_mb: Some(memory_max_mb),
        fuel: Some(fuel),
        epoch_ms: Some(epoch_ms),
        replicas,
        start,
        binary_sha256_hex: digest,
    };
    let unsigned_bytes = serde_json::to_vec(&unsigned)?;
    let sig = sign_bytes_ed25519(&kp.private_hex, &unsigned_bytes)?;
    let pkg = PushPackage {
        unsigned,
        binary_b64: base64::engine::general_purpose::STANDARD.encode(&bin),
        signature_b64: base64::engine::general_purpose::STANDARD.encode(sig),
    };

    libp2p::Swarm::behaviour_mut(&mut swarm)
        .gossipsub
        .publish(topic_cmd.clone(), serialize_message(&Command::PushComponent(pkg)))?;

    // brief wait to let it propagate
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    Ok(())
}
