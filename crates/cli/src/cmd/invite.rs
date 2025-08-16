use anyhow::Context;
use base64::Engine;

use common::{InviteToken, InviteUnsigned, OwnerKeypair, sign_bytes_ed25519};

use super::util::owner_dir;

pub async fn invite(bootstrap: Vec<String>, realm_id: Option<String>, exp_mins: u64) -> anyhow::Result<()> {
    // load owner key
    let dir = owner_dir()?;
    let key_path = dir.join("owner.key.json");
    let bytes = tokio::fs::read(&key_path).await.context("read owner key")?;
    let kp: OwnerKeypair = serde_json::from_slice(&bytes)?;

    let exp_unix = if exp_mins == 0 {
        None
    } else {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
        Some(now + exp_mins * 60)
    };
    let unsigned = InviteUnsigned {
        alg: "ed25519".into(),
        owner_pub_bs58: kp.public_bs58.clone(),
        bootstrap_multiaddrs: bootstrap,
        realm_id,
        exp_unix,
    };
    let unsigned_bytes = serde_json::to_vec(&unsigned)?;
    let sig = sign_bytes_ed25519(&kp.private_hex, &unsigned_bytes)?;
    let token = InviteToken { unsigned, signature_b64: base64::engine::general_purpose::STANDARD.encode(sig) };
    let token_json = serde_json::to_vec(&token)?;
    let token_b64 = base64::engine::general_purpose::STANDARD.encode(token_json);
    println!("{}", token_b64);
    Ok(())
}

