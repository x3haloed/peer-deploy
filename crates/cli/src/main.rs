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

use common::{serialize_message, Command, REALM_CMD_TOPIC, REALM_STATUS_TOPIC, OwnerKeypair, SignedManifest, sign_bytes_ed25519};

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
            signature_b64: base64::encode(sig),
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
