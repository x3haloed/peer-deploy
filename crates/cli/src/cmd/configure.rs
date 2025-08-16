use anyhow::anyhow;

use super::util::{write_trusted_owner, write_bootstrap};

pub async fn configure(owner: String, bootstrap: Vec<String>) -> anyhow::Result<()> {
    if owner.is_empty() {
        return Err(anyhow!("owner key required"));
    }
    write_trusted_owner(&owner).await?;
    write_bootstrap(&bootstrap).await?;
    println!("configuration written");
    Ok(())
}

