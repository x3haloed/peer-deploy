use anyhow::anyhow;
use base64::Engine;

use common::{verify_bytes_ed25519, InviteToken};

use super::util::{write_bootstrap, write_trusted_owner};

pub async fn enroll(token_b64: String, binary: Option<String>, system: bool) -> anyhow::Result<()> {
    let token_bytes = base64::engine::general_purpose::STANDARD.decode(token_b64)?;
    let token: InviteToken = serde_json::from_slice(&token_bytes)?;

    // verify signature
    let unsigned_bytes = serde_json::to_vec(&token.unsigned)?;
    let sig = base64::engine::general_purpose::STANDARD.decode(&token.signature_b64)?;
    let ok = verify_bytes_ed25519(&token.unsigned.owner_pub_bs58, &unsigned_bytes, &sig)?;
    if !ok {
        return Err(anyhow!("invalid invite signature"));
    }

    if let Some(exp) = token.unsigned.exp_unix {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        if now > exp {
            return Err(anyhow!("invite token expired"));
        }
    }

    write_trusted_owner(&token.unsigned.owner_pub_bs58).await?;
    write_bootstrap(&token.unsigned.bootstrap_multiaddrs).await?;

    if let Some(bin) = binary {
        #[cfg(unix)]
        {
            return super::install::install(Some(bin), system).await;
        }
        #[cfg(not(unix))]
        {
            println!("configured. please install the agent binary separately.");
            return Ok(());
        }
    }

    println!("configured. run 'realm install --binary <path>' to install the agent.");
    Ok(())
}
