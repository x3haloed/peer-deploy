use anyhow::Context;

use common::{sha256_hex, sign_bytes_ed25519, serialize_message, Command, OwnerKeypair, PushPackage, PushUnsigned, Visibility};

use super::util::{mdns_warmup, new_swarm, owner_dir};
use base64::Engine;

pub async fn deploy_component(
    path: String,
    package: Option<String>,
    profile: String,
    features: String,
    target_peers: Vec<String>,
    target_tags: Vec<String>,
    name_override: Option<String>,
    start: bool,
) -> anyhow::Result<()> {
    // Ensure cargo-component is installed
    let status = std::process::Command::new("cargo")
        .args(["component", "--version"])
        .status()
        .context("check cargo-component installed")?;
    if !status.success() {
        anyhow::bail!("cargo-component not found; install with: cargo install cargo-component");
    }

    let pkg = package.unwrap_or_else(|| String::new());
    let mut args = vec!["component", "build", "--target", "wasm32-wasip1"];
    if profile == "release" { args.push("--release"); }
    if !features.is_empty() { args.extend(["-F", &features]); }
    if !pkg.is_empty() { args.extend(["-p", &pkg]); }

    let mut cmd = std::process::Command::new("cargo");
    cmd.current_dir(&path);
    cmd.args(args);
    let status = cmd.status().context("cargo component build")?;
    if !status.success() { anyhow::bail!("cargo component build failed"); }

    // Locate artifact path
    let prof = if profile == "release" { "release" } else { "debug" };
    let pkg_name = if pkg.is_empty() {
        // read Cargo.toml package.name
        let toml_path = std::path::Path::new(&path).join("Cargo.toml");
        let text = std::fs::read_to_string(&toml_path).context("read Cargo.toml")?;
        let value: toml::Value = toml::from_str(&text)?;
        value.get("package").and_then(|t| t.get("name")).and_then(|n| n.as_str()).unwrap_or("component").to_string()
    } else { pkg.clone() };
    let artifact = std::path::Path::new(&path)
        .join("target/wasm32-wasip1")
        .join(prof)
        .join(format!("{}.wasm", pkg_name.replace('-', "_")));
    if !artifact.exists() {
        anyhow::bail!(format!("artifact not found: {}", artifact.display()));
    }

    // Set a default deployment name
    let deploy_name = name_override.unwrap_or_else(|| pkg_name.clone());

    // Prepare P2P
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

    // Read wasm and sign
    let bin = tokio::fs::read(&artifact).await.context("read wasm artifact")?;
    let digest = sha256_hex(&bin);
    let unsigned = PushUnsigned {
        alg: "ed25519".into(),
        owner_pub_bs58: kp.public_bs58.clone(),
        component_name: deploy_name,
        target_peer_ids: target_peers,
        target_tags,
        memory_max_mb: Some(64),
        fuel: Some(5_000_000),
        epoch_ms: Some(100),
        replicas: 1,
        start,
        binary_sha256_hex: digest,
        mounts: None,
        ports: None,
        visibility: Some(Visibility::Local),
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

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    Ok(())
}


