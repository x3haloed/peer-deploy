use anyhow::Context;

use super::util::owner_dir;
use common::sha256_hex;

pub async fn init() -> anyhow::Result<()> {
    let dir = owner_dir()?;
    tokio::fs::create_dir_all(&dir).await?;

    let key_path = dir.join("owner.key.json");
    if tokio::fs::try_exists(&key_path).await? {
        println!("owner key already exists at {}", key_path.display());
        return Ok(());
    }

    let kp = common::OwnerKeypair::generate()?;
    let json = serde_json::to_vec_pretty(&kp)?;
    tokio::fs::write(&key_path, json).await?;
    println!("initialized; owner pub: {}", kp.public_bs58);

    // Generate a sample realm.toml pointing to hello.wasm if present
    let hello_path = std::path::Path::new("target/wasm32-wasip1/debug/hello.wasm");
    if hello_path.exists() {
        let bytes = tokio::fs::read(hello_path).await.unwrap_or_default();
        let digest = sha256_hex(&bytes);
        let sample = format!(
            "[components.hello]\nsource = \"file:{}\"\nsha256_hex = \"{}\"\nmemory_max_mb = 64\nfuel = 5000000\nepoch_ms = 100\n",
            hello_path.display(), digest
        );
        let sample_path = dir.join("realm.sample.toml");
        tokio::fs::write(&sample_path, sample).await.ok();
        println!("wrote sample manifest at {}", sample_path.display());
    }
    Ok(())
}

pub async fn key_show() -> anyhow::Result<()> {
    let key_path = owner_dir()?.join("owner.key.json");
    let bytes = tokio::fs::read(&key_path).await.context("read owner key")?;
    let kp: common::OwnerKeypair = serde_json::from_slice(&bytes)?;
    println!("{}", kp.public_bs58);
    Ok(())
}

