use anyhow::Context;

use super::util::{owner_dir, read_trusted_owner, agent_data_dir_cli};

pub async fn whoami() -> anyhow::Result<()> {
    // Owner key (CLI/TUI identity for signing)
    let owner_path = owner_dir()?.join("owner.key.json");
    let owner_pub = match tokio::fs::read(&owner_path).await {
        Ok(bytes) => {
            let kp: common::OwnerKeypair = serde_json::from_slice(&bytes).context("parse owner.key.json")?;
            Some(kp.public_bs58)
        }
        Err(_) => None,
    };

    // Agent trusted owner (who this machine accepts commands from)
    let trusted_owner = read_trusted_owner().await?;

    // Agent node identity (PeerId) if available via stateful file
    let peer_id = match tokio::fs::read(agent_data_dir_cli()?.join("node.peer")).await {
        Ok(bytes) => Some(String::from_utf8_lossy(&bytes).trim().to_string()),
        Err(_) => None,
    };

    println!("CLI owner pub: {}", owner_pub.as_deref().unwrap_or("<missing>"));
    println!(
        "Agent trusted owner: {}",
        trusted_owner.as_deref().unwrap_or("<unset; TOFU on first signed command>")
    );
    if let Some(pid) = peer_id {
        println!("Agent peer id: {}", pid);
    }
    Ok(())
}


