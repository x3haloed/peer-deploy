use std::time::Duration;

use anyhow::{Context, anyhow};
use clap::{Parser, Subcommand};
use futures::StreamExt;
use libp2p::{
    gossipsub, mdns,
    swarm::{Swarm, SwarmEvent},
    Multiaddr, PeerId, SwarmBuilder,
};
use tracing_subscriber::EnvFilter;
use base64::Engine;

use common::{serialize_message, Command, REALM_CMD_TOPIC, REALM_STATUS_TOPIC, OwnerKeypair, SignedManifest, AgentUpgrade, sign_bytes_ed25519};
use common::sha256_hex;

#[derive(Debug, Parser)]
#[command(name = "realm")]
#[command(about = "peer-deploy CLI", version, author)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Generate local owner key
    Init,
    /// Display owner public key
    KeyShow,
    /// Send a hello to the network or run a wasm
    Apply {
        /// Optional wasm path to instruct agents to run (ad-hoc)
        #[arg(long)]
        wasm: Option<String>,
        /// Optional path to realm.toml to sign + publish
        #[arg(long)]
        file: Option<String>,
        /// Monotonic version for this manifest
        #[arg(long, default_value_t = 1)]
        version: u64,
    },
    /// Query status from agents and print first reply
    Status,
    /// Install the agent as a systemd user service
    Install {
        /// Path to the agent binary to install
        #[arg(long)]
        binary: String,
    },
    /// Push an agent binary upgrade to all peers
    Upgrade {
        /// Path to agent binary to distribute
        #[arg(long)]
        file: String,
        /// Monotonic version for the agent binary
        #[arg(long, default_value_t = 1)]
        version: u64,
    },
}

#[derive(libp2p::swarm::NetworkBehaviour)]
struct NodeBehaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::tokio::Behaviour,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init => init().await,
        Commands::KeyShow => key_show().await,
        Commands::Apply { wasm, file, version } => apply(wasm, file, version).await,
        Commands::Status => status().await,
        Commands::Install { binary } => install(binary).await,
        Commands::Upgrade { file, version } => upgrade(file, version).await,
    }
}

fn owner_dir() -> anyhow::Result<std::path::PathBuf> {
    Ok(dirs::config_dir().context("config dir")?.join("realm"))
}

async fn init() -> anyhow::Result<()> {
    let dir = owner_dir()?;
    tokio::fs::create_dir_all(&dir).await?;

    let key_path = dir.join("owner.key.json");
    if tokio::fs::try_exists(&key_path).await? {
        println!("owner key already exists at {}", key_path.display());
        return Ok(());
    }

    let kp = OwnerKeypair::generate()?;
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

async fn key_show() -> anyhow::Result<()> {
    let dir = owner_dir()?;
    let key_path = dir.join("owner.key.json");
    let bytes = tokio::fs::read(&key_path).await.context("read owner key")?;
    let kp: OwnerKeypair = serde_json::from_slice(&bytes)?;
    println!("{}", kp.public_bs58);
    Ok(())
}

async fn new_swarm() -> anyhow::Result<(Swarm<NodeBehaviour>, gossipsub::IdentTopic, gossipsub::IdentTopic)> {
    let id_keys = libp2p::identity::Keypair::generate_ed25519();

    let gossip_config = gossipsub::ConfigBuilder::default().build()?;
    let mut gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys.clone()),
        gossip_config,
    ).map_err(|e| anyhow!(e))?;

    let topic_cmd = gossipsub::IdentTopic::new(REALM_CMD_TOPIC);
    let topic_status = gossipsub::IdentTopic::new(REALM_STATUS_TOPIC);
    gossipsub.subscribe(&topic_cmd)?;
    gossipsub.subscribe(&topic_status)?;

    let mdns_beh = mdns::tokio::Behaviour::new(mdns::Config::default(), PeerId::from(id_keys.public()))?;

    let behaviour = NodeBehaviour { gossipsub, mdns: mdns_beh };

    let swarm = SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_quic()
        .with_dns()?
        .with_behaviour(|_| Ok(behaviour))?
        .build();

    Ok((swarm, topic_cmd, topic_status))
}

async fn apply(wasm: Option<String>, file: Option<String>, version: u64) -> anyhow::Result<()> {
    let (mut swarm, topic_cmd, _topic_status) = new_swarm().await?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/udp/0/quic-v1".parse::<Multiaddr>().unwrap())?;

    // Brief mDNS warmup
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(600) {
        if let Some(event) = swarm.next().await {
            if let SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(ev)) = event {
                match ev {
                    mdns::Event::Discovered(list) => {
                        for (peer, _addr) in list { swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer); }
                    }
                    mdns::Event::Expired(list) => {
                        for (peer, _addr) in list { swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer); }
                    }
                }
            }
        }
    }

    // ad-hoc hello/run path still supported
    let hello = Command::Hello { from: hostname::get().unwrap_or_default().to_string_lossy().into_owned() };
    swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), serialize_message(&hello))?;

    if let Some(path) = wasm {
        let run = Command::Run { wasm_path: path, memory_max_mb: 64, fuel: 5_000_000, epoch_ms: 100 };
        swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), serialize_message(&run))?;
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
        swarm.behaviour_mut().gossipsub.publish(topic_cmd.clone(), serialize_message(&Command::ApplyManifest(signed)))?;
    }

    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(())
}

async fn status() -> anyhow::Result<()> {
    let (mut swarm, topic_cmd, topic_status) = new_swarm().await?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/udp/0/quic-v1".parse::<Multiaddr>().unwrap())?;

    swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic_cmd.clone(), serialize_message(&Command::StatusQuery))?;

    let timeout = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match swarm.select_next_some().await {
                SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(ev)) => {
                    match ev {
                        mdns::Event::Discovered(list) => {
                            for (peer, _addr) in list { swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer); }
                        }
                        mdns::Event::Expired(list) => {
                            for (peer, _addr) in list { swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer); }
                        }
                    }
                }
                SwarmEvent::Behaviour(NodeBehaviourEvent::Gossipsub(ev)) => {
                    if let gossipsub::Event::Message { message, .. } = ev {
                        if message.topic == topic_status.hash() {
                            println!("{}", String::from_utf8_lossy(&message.data));
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
    });

    match timeout.await {
        Ok(_) => Ok(()),
        Err(_) => {
            println!("no status received");
            Ok(())
        }
    }
}

async fn install(binary: String) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let bin_dir = dirs::home_dir().context("home dir")?.join(".local/bin");
    tokio::fs::create_dir_all(&bin_dir).await?;
    let target = bin_dir.join("realm-agent");
    tokio::fs::copy(&binary, &target).await?;
    tokio::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).await?;

    let systemd_dir = dirs::config_dir().context("config dir")?.join("systemd/user");
    tokio::fs::create_dir_all(&systemd_dir).await?;
    let service_path = systemd_dir.join("realm-agent.service");
    let service = format!("[Unit]\nDescription=Realm Agent\n\n[Service]\nExecStart={}\nRestart=always\n\n[Install]\nWantedBy=default.target\n", target.display());
    tokio::fs::write(&service_path, service).await?;

    if std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status().is_ok() {
        let _ = std::process::Command::new("systemctl").args(["--user", "enable", "--now", "realm-agent"]).status();
        println!("installed and started systemd user service realm-agent");
    } else {
        println!("service file written to {}. enable with: systemctl --user enable --now realm-agent", service_path.display());
    }
    Ok(())
}

async fn upgrade(file: String, version: u64) -> anyhow::Result<()> {
    let (mut swarm, topic_cmd, _topic_status) = new_swarm().await?;
    libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/udp/0/quic-v1".parse::<Multiaddr>().unwrap())?;

    // mDNS warmup like apply
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(600) {
        if let Some(event) = swarm.next().await {
            if let SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(ev)) = event {
                match ev {
                    mdns::Event::Discovered(list) => {
                        for (peer, _addr) in list { swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer); }
                    }
                    mdns::Event::Expired(list) => {
                        for (peer, _addr) in list { swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer); }
                    }
                }
            }
        }
    }

    // load owner key
    let dir = owner_dir()?;
    let key_path = dir.join("owner.key.json");
    let bytes = tokio::fs::read(&key_path).await.context("read owner key")?;
    let kp: OwnerKeypair = serde_json::from_slice(&bytes)?;

    let bin_bytes = tokio::fs::read(&file).await?;
    let digest = sha256_hex(&bin_bytes);
    let sig = sign_bytes_ed25519(&kp.private_hex, &bin_bytes)?;
    let pkg = AgentUpgrade {
        alg: "ed25519".into(),
        owner_pub_bs58: kp.public_bs58.clone(),
        version,
        binary_sha256_hex: digest,
        binary_b64: base64::engine::general_purpose::STANDARD.encode(&bin_bytes),
        signature_b64: base64::engine::general_purpose::STANDARD.encode(sig),
    };
    swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic_cmd.clone(), serialize_message(&Command::UpgradeAgent(pkg)))?;

    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(())
}
