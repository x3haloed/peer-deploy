use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use realm::{tui, cmd};

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
        /// Static route (repeatable): path=/web[,host=app.local],dir=/abs/dir
        #[arg(long = "route-static")]
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
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();

    // If no args, launch TUI as default operating mode
    if std::env::args().len() == 1 {
        return tui::run_tui().await;
    }

    let cli = Cli::parse();

    match cli.command {
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
    }
}

