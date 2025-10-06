use anyhow::Context;

use common::{
    serialize_message, sha256_hex, sign_bytes_ed25519, Command, OwnerKeypair, PushPackage,
    PushUnsigned, Visibility,
};

use super::util::{dial_bootstrap, mdns_warmup, new_swarm, owner_dir};
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
    if profile == "release" {
        args.push("--release");
    }
    if !features.is_empty() {
        args.extend(["-F", &features]);
    }
    if !pkg.is_empty() {
        args.extend(["-p", &pkg]);
    }

    let mut cmd = std::process::Command::new("cargo");
    cmd.current_dir(&path);
    cmd.args(args);
    let status = cmd.status().context("cargo component build")?;
    if !status.success() {
        anyhow::bail!("cargo component build failed");
    }

    // Locate artifact path
    let prof = if profile == "release" {
        "release"
    } else {
        "debug"
    };
    let pkg_name = if pkg.is_empty() {
        // read Cargo.toml package.name
        let toml_path = std::path::Path::new(&path).join("Cargo.toml");
        let text = std::fs::read_to_string(&toml_path).context("read Cargo.toml")?;
        let value: toml::Value = toml::from_str(&text)?;
        value
            .get("package")
            .and_then(|t| t.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("component")
            .to_string()
    } else {
        pkg.clone()
    };
    let filename = format!("{}.wasm", pkg_name.replace('-', "_"));
    let rel = std::path::Path::new("wasm32-wasip1")
        .join(prof)
        .join(&filename);

    // Candidate 1: component-local target dir
    let candidate_local = std::path::Path::new(&path).join("target").join(&rel);
    // Candidate 2: workspace/global target dir (respects CARGO_TARGET_DIR if set)
    let target_root = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    let candidate_workspace = std::path::Path::new(&target_root).join(&rel);

    let artifact = if candidate_local.exists() {
        candidate_local
    } else if candidate_workspace.exists() {
        candidate_workspace
    } else {
        anyhow::bail!(format!(
            "artifact not found. looked for: {} and {}",
            candidate_local.display(),
            candidate_workspace.display()
        ));
    };

    // Set a default deployment name
    let deploy_name = name_override.unwrap_or_else(|| pkg_name.clone());

    // Prepare P2P
    let (mut swarm, topic_cmd, _topic_status) = new_swarm().await?;
    libp2p::Swarm::listen_on(
        &mut swarm,
        "/ip4/0.0.0.0/udp/0/quic-v1"
            .parse::<libp2p::Multiaddr>()
            .map_err(|e| anyhow::anyhow!("Failed to parse multiaddr: {}", e))?,
    )?;
    mdns_warmup(&mut swarm).await;
    dial_bootstrap(&mut swarm).await;

    // load owner key
    let dir = owner_dir()?;
    let key_path = dir.join("owner.key.json");
    let bytes = tokio::fs::read(&key_path).await.context("read owner key")?;
    let kp: OwnerKeypair = serde_json::from_slice(&bytes)?;

    // Read wasm and sign
    let bin = tokio::fs::read(&artifact)
        .await
        .context("read wasm artifact")?;
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

    // Wait for stable connections and then send commands with retry logic
    use futures::StreamExt;
    let start = std::time::Instant::now();
    let connect_deadline = std::time::Duration::from_secs(5);
    let mut connected_peers: u32 = 0;
    let mut command_sent = false;
    let mut republish = tokio::time::interval(std::time::Duration::from_millis(2000));

    // First wait for at least one connection
    println!("Establishing P2P connections...");
    loop {
        if start.elapsed() >= connect_deadline {
            if connected_peers == 0 {
                println!("Warning: No P2P connections established, sending command anyway...");
            }
            break;
        }
        tokio::select! {
            _ = republish.tick() => {
                if connected_peers > 0 && !command_sent {
                    println!("Sending deployment command to {} connected peer(s)...", connected_peers);
                    let _ = libp2p::Swarm::behaviour_mut(&mut swarm)
                        .gossipsub
                        .publish(topic_cmd.clone(), serialize_message(&Command::PushComponent(pkg.clone())));
                    command_sent = true;
                } else if command_sent {
                    // Reduce republishing frequency after initial success
                    break;
                }
            }
            Some(event) = swarm.next() => {
                match event {
                    libp2p::swarm::SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        connected_peers += 1;
                        println!("Connected to peer: {}", peer_id);
                    }
                    libp2p::swarm::SwarmEvent::ConnectionClosed { .. } => {
                        connected_peers = connected_peers.saturating_sub(1);
                    }
                    libp2p::swarm::SwarmEvent::Behaviour(super::util::NodeBehaviourEvent::Mdns(ev)) => {
                        match ev {
                            libp2p::mdns::Event::Discovered(list) => {
                                for (peer, _addr) in list {
                                    libp2p::Swarm::behaviour_mut(&mut swarm).gossipsub.add_explicit_peer(&peer);
                                }
                            }
                            libp2p::mdns::Event::Expired(list) => {
                                for (peer, _addr) in list {
                                    libp2p::Swarm::behaviour_mut(&mut swarm).gossipsub.remove_explicit_peer(&peer);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            else => { break; }
        }
    }

    // Continue sending for a bit longer to ensure delivery
    if !command_sent {
        println!("Sending deployment command...");
        let _ = libp2p::Swarm::behaviour_mut(&mut swarm).gossipsub.publish(
            topic_cmd.clone(),
            serialize_message(&Command::PushComponent(pkg.clone())),
        );
    }

    // Send command one more time after a short delay for extra reliability
    if command_sent {
        tokio::time::sleep(std::time::Duration::from_millis(3000)).await;
        let _ = libp2p::Swarm::behaviour_mut(&mut swarm).gossipsub.publish(
            topic_cmd.clone(),
            serialize_message(&Command::PushComponent(pkg.clone())),
        );
    }

    println!("Deployment command sent. Check agent status with: cargo run -p realm -- status");
    Ok(())
}
