use std::time::Duration;

use anyhow::Context;
use base64::Engine;

use common::{serialize_message, sign_bytes_ed25519, Command, OwnerKeypair, SignedManifest};

use super::util::{mdns_warmup, new_swarm, owner_dir};

pub async fn apply(wasm: Option<String>, file: Option<String>, version: u64) -> anyhow::Result<()> {
    let (mut swarm, topic_cmd, _topic_status) = new_swarm().await?;
    libp2p::Swarm::listen_on(
        &mut swarm,
        "/ip4/0.0.0.0/udp/0/quic-v1"
            .parse::<libp2p::Multiaddr>()
            .unwrap(),
    )?;

    mdns_warmup(&mut swarm).await;

    // ad-hoc hello/run path still supported
    let hello = Command::Hello {
        from: hostname::get()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    };
    swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic_cmd.clone(), serialize_message(&hello))?;

    if let Some(path) = wasm {
        let run = Command::Run {
            wasm_path: path,
            memory_max_mb: 64,
            fuel: 5_000_000,
            epoch_ms: 100,
        };
        swarm
            .behaviour_mut()
            .gossipsub
            .publish(topic_cmd.clone(), serialize_message(&run))?;
    }

    if let Some(manifest_path) = file {
        let toml_bytes = tokio::fs::read(&manifest_path).await?;
        let toml_str = String::from_utf8(toml_bytes.clone()).context("realm.toml utf8")?;
        // load owner key
        let dir = owner_dir()?;
        let key_path = dir.join("owner.key.json");
        let bytes = tokio::fs::read(&key_path).await.context("read owner key")?;
        let kp: OwnerKeypair = serde_json::from_slice(&bytes)?;
        let sig = sign_bytes_ed25519(&kp.private_hex, toml_bytes.as_slice())?;
        let signed = SignedManifest {
            alg: "ed25519".into(),
            owner_pub_bs58: kp.public_bs58.clone(),
            version,
            manifest_toml: toml_str,
            signature_b64: base64::engine::general_purpose::STANDARD.encode(sig),
        };
        swarm.behaviour_mut().gossipsub.publish(
            topic_cmd.clone(),
            serialize_message(&Command::ApplyManifest(signed)),
        )?;
    }

    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(())
}
