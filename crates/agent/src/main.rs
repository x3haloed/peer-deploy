mod p2p;
mod runner;
mod supervisor;
mod cmd;
mod tui;
mod web;

use clap::{Parser, Subcommand};
use tracing::{info};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "realm")]
#[command(about = "peer-deploy unified agent and CLI", version, author)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run the agent (default mode when no subcommand specified)
    Agent {
        /// Optional WASM module path to run at startup
        #[arg(long)]
        wasm: Option<String>,

        /// Maximum memory in MB for the WASM instance
        #[arg(long, default_value_t = 64)]
        memory_max_mb: u64,

        /// Initial fuel units to provide to WASM
        #[arg(long, default_value_t = 5_000_000)]
        fuel: u64,

        /// Epoch deadline interval in milliseconds
        #[arg(long, default_value_t = 100)]
        epoch_ms: u64,

        /// Roles/tags this agent advertises (repeat flag for multiple)
        #[arg(long = "role")]
        roles: Vec<String>,
    },
    /// Generate local owner key
    Init,
    /// Display owner public key
    KeyShow,
    /// Send a hello to the network or run a wasm / publish manifest
    Apply {
        #[arg(long)]
        wasm: Option<String>,
        #[arg(long)]
        file: Option<String>,
        #[arg(long, default_value_t = 1)]
        version: u64,
    },
    /// Query status from agents and print first reply
    Status,
    /// Install the agent as a service (systemd user service by default)
    Install {
        #[arg(long)]
        binary: Option<String>,
        #[arg(long, default_value_t = false)]
        system: bool,
    },
    /// Push an agent binary upgrade to peers
    Upgrade {
        #[arg(long)]
        file: String,
        #[arg(long, default_value_t = 1)]
        version: u64,
        /// Target specific peers by PeerId (repeatable)
        #[arg(long = "peer")]
        target_peers: Vec<String>,
        /// Target peers with any of these tags/roles (repeatable)
        #[arg(long = "tag")]
        target_tags: Vec<String>,
    },
    /// Push a WASI component to selected peers and optionally start it
    Push {
        #[arg(long)]
        name: String,
        #[arg(long)]
        file: String,
        #[arg(long, default_value_t = 1)]
        replicas: u32,
        #[arg(long, default_value_t = 64)]
        memory_max_mb: u64,
        #[arg(long, default_value_t = 5_000_000)]
        fuel: u64,
        #[arg(long, default_value_t = 100)]
        epoch_ms: u64,
        /// Preopen mounts (repeatable): host=/abs/path,guest=/www[,ro=true]
        #[arg(long = "mount")]
        mounts: Vec<String>,
        /// Declare service ports (repeatable), e.g. 8080/tcp or 9090/udp
        #[arg(long = "port")]
        ports: Vec<String>,
        /// [deprecated] Static routes removed; HTTP is handled via WASI HTTP inside components.
        #[arg(long = "route-static", hide = true)]
        routes_static: Vec<String>,
        /// Gateway bind policy: local|public
        #[arg(long)]
        visibility: Option<String>,
        /// Target specific peers by PeerId (repeatable)
        #[arg(long = "peer")]
        target_peers: Vec<String>,
        /// Target peers with any of these tags/roles (repeatable)
        #[arg(long = "tag")]
        target_tags: Vec<String>,
        /// Stage only; don't start
        #[arg(long, default_value_t = true)]
        start: bool,
    },
    /// Create a signed invite token for bootstrapping a new peer
    Invite {
        #[arg(long)]
        bootstrap: Vec<String>,
        #[arg(long)]
        realm_id: Option<String>,
        #[arg(long, default_value_t = 60)]
        exp_mins: u64,
    },
    /// Enroll a new peer using an invite token; optionally install the agent
    Enroll {
        #[arg(long)]
        token: String,
        #[arg(long)]
        binary: Option<String>,
        #[arg(long, default_value_t = false)]
        system: bool,
    },
    /// Manually configure trust and bootstrap peers
    Configure {
        #[arg(long)]
        owner: String,
        #[arg(long)]
        bootstrap: Vec<String>,
    },
    /// Diagnose a QUIC dial attempt to a multiaddr; prints handshake results
    DiagQuic {
        #[arg(value_name = "MULTIADDR")]
        addr: String,
    },
    /// Print identities: CLI owner key, agent trusted owner, agent PeerId
    Whoami,
    /// Build a cargo-component and push to agents
    DeployComponent {
        /// Path to the cargo project directory (containing Cargo.toml)
        #[arg(long, value_name = "DIR", default_value = ".")]
        path: String,
        /// Component package name (Cargo package name)
        #[arg(long)]
        package: Option<String>,
        /// Build profile: debug or release
        #[arg(long, default_value = "release")]
        profile: String,
        /// Additional cargo features (comma-separated)
        #[arg(long, default_value = "component")]
        features: String,
        /// Target peers by PeerId (repeatable)
        #[arg(long = "peer")]
        target_peers: Vec<String>,
        /// Target peers by tag/role (repeatable)
        #[arg(long = "tag")]
        target_tags: Vec<String>,
        /// Component name for deployment (defaults to package name)
        #[arg(long)]
        name: Option<String>,
        /// Start immediately after push
        #[arg(long, default_value_t = true)]
        start: bool,
    },
    /// Launch the TUI interface
    Tui,
    /// Start management web interface
    Manage {
        /// Authentication method (for now, always authenticates)
        #[arg(long)]
        owner_key: bool,
        /// Session timeout in minutes
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();

    // Default behavior: if no subcommand specified, start TUI
    let command = cli.command.unwrap_or_else(|| {
        // If invoked as "realm-agent" with no args, default to agent mode
        // If invoked as just CLI commands, default to TUI
        Commands::Tui
    });

    match command {
        Commands::Agent { wasm, memory_max_mb, fuel, epoch_ms, roles } => {
            let shutdown = setup_shutdown_handler();
            tokio::select! {
                result = p2p::run_agent(wasm, memory_max_mb, fuel, epoch_ms, roles) => result,
                _ = shutdown => {
                    info!("Shutdown signal received, stopping agent gracefully");
                    Ok(())
                }
            }
        },
        Commands::Tui => {
            let shutdown = setup_shutdown_handler();
            tokio::select! {
                result = tui::run_tui() => result,
                _ = shutdown => Ok(())
            }
        },
        Commands::Init => cmd::init().await,
        Commands::KeyShow => cmd::key_show().await,
        Commands::Apply { wasm, file, version } => cmd::apply(wasm, file, version).await,
        Commands::Status => cmd::status().await,
        #[cfg(unix)]
        Commands::Install { binary, system } => cmd::install(binary, system).await,
        #[cfg(not(unix))]
        Commands::Install { .. } => Err(anyhow::anyhow!("install is only supported on Unix-like systems with systemd")),
        Commands::Upgrade { file, version, target_peers, target_tags } =>
            cmd::upgrade(file, version, target_peers, target_tags).await,
        Commands::Invite { bootstrap, realm_id, exp_mins } => cmd::invite(bootstrap, realm_id, exp_mins).await,
        Commands::Enroll { token, binary, system } => cmd::enroll(token, binary, system).await,
        Commands::Configure { owner, bootstrap } => cmd::configure(owner, bootstrap).await,
        Commands::DiagQuic { addr } => cmd::diag_quic(addr).await,
        Commands::Whoami => cmd::whoami().await,
        Commands::Push { name, file, replicas, memory_max_mb, fuel, epoch_ms, mounts, ports, routes_static, visibility, target_peers, target_tags, start } => cmd::push(name, file, replicas, memory_max_mb, fuel, epoch_ms, mounts, ports, routes_static, visibility, target_peers, target_tags, start).await,
        Commands::DeployComponent { path, package, profile, features, target_peers, target_tags, name, start } => cmd::deploy_component(path, package, profile, features, target_peers, target_tags, name, start).await,
        Commands::Manage { owner_key, timeout } => {
            use std::time::Duration;
            let timeout_duration = Duration::from_secs(timeout * 60);
            web::start_management_session(owner_key, timeout_duration).await
        },
    }
}

async fn setup_shutdown_handler() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
