use anyhow::Context;
use base64::Engine;

use common::{sha256_hex, sign_bytes_ed25519, serialize_message, Command, OwnerKeypair, PushPackage, PushUnsigned, MountSpec, ServicePort, Protocol, Visibility};

use super::util::{mdns_warmup, new_swarm, owner_dir};

pub async fn push(
    name: String,
    file: String,
    replicas: u32,
    memory_max_mb: u64,
    fuel: u64,
    epoch_ms: u64,
    mounts_cli: Vec<String>,
    ports_cli: Vec<String>,
    _routes_static_cli: Vec<String>,
    visibility_cli: Option<String>,
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

    // Parse mounts from CLI strings host=...,guest=...[,ro=true]
    let mut mounts: Option<Vec<MountSpec>> = None;
    if !mounts_cli.is_empty() {
        let mut list = Vec::new();
        for m in mounts_cli.iter() {
            let mut host: Option<String> = None;
            let mut guest: Option<String> = None;
            let mut ro = false;
            for part in m.split(',') {
                let mut it = part.splitn(2, '=');
                if let (Some(k), Some(v)) = (it.next(), it.next()) {
                    match k.trim() {
                        "host" => host = Some(v.trim().to_string()),
                        "guest" => guest = Some(v.trim().to_string()),
                        "ro" => ro = v.trim().eq_ignore_ascii_case("true"),
                        _ => {}
                    }
                }
            }
            if let (Some(h), Some(g)) = (host, guest) {
                list.push(MountSpec { host: h, guest: g, ro });
            }
        }
        if !list.is_empty() { mounts = Some(list); }
    }

    // Parse ports 8080/tcp style
    let ports: Option<Vec<ServicePort>> = if ports_cli.is_empty() { None } else {
        let mut out = Vec::new();
        for p in ports_cli.iter() {
            let mut it = p.split('/');
            if let (Some(num), Some(proto)) = (it.next(), it.next()) {
                if let Ok(port) = num.parse::<u16>() {
                    let protocol = if proto.eq_ignore_ascii_case("udp") { Protocol::Udp } else { Protocol::Tcp };
                    out.push(ServicePort { name: None, port, protocol });
                }
            }
        }
        if out.is_empty() { None } else { Some(out) }
    };

    // Static routes removed; WASI HTTP handles requests inside components now.

    let visibility = visibility_cli.and_then(|v| match v.as_str() {
        "local" | "Local" => Some(Visibility::Local),
        "public" | "Public" => Some(Visibility::Public),
        _ => None,
    });

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
        mounts,
        ports,
        visibility,
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

/// Deploy a .realm package via CLI by calling the same internal installer as the web API.
pub async fn push_package(file: String, name_override: Option<String>) -> anyhow::Result<()> {
    let bytes = tokio::fs::read(&file).await.context("read package")?;
    // Connect to management web state directly (local agent)
    let state = crate::web::connect_to_agent().await?;
    let (_name, _digest) = crate::web::install_package_from_bytes(&state, bytes, name_override).await
        .map_err(|e: String| anyhow::anyhow!(e))?;
    println!("Package deployed");
    Ok(())
}
