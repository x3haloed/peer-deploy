use anyhow::anyhow;

use super::util::{agent_data_dir_cli, write_bootstrap, write_trusted_owner};

pub async fn configure(owner: String, bootstrap: Vec<String>) -> anyhow::Result<()> {
    if owner.is_empty() {
        return Err(anyhow!("owner key required"));
    }
    write_trusted_owner(&owner).await?;
    write_bootstrap(&bootstrap).await?;
    // If user passed a listen port via BOOTSTRAP like /udp/<port>/..., we don't parse it here.
    // For explicit port configuration, allow an env var for now: REALM_LISTEN_PORT
    if let Ok(port_str) = std::env::var("REALM_LISTEN_PORT") {
        if let Ok(port) = port_str.parse::<u16>() {
            let path = agent_data_dir_cli()?.join("listen_port");
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
            tokio::fs::write(path, port.to_string()).await.ok();
            println!("set listen port to {} (persisted)", port);
        }
    }
    println!("configuration written");
    Ok(())
}
