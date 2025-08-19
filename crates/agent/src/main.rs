mod p2p;
mod runner;
mod supervisor;
mod cmd;
mod web;
mod job_manager;
mod policy;
mod storage;

use clap::{Parser, Subcommand};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "realm")]
#[command(about = "peer-deploy unified agent and CLI", version, author)]
struct Cli {
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

    #[command(subcommand)]
    command: Option<Commands>,
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
        /// Repeatable: specify one or more platform=path pairs (e.g., --bin linux/x86_64=./agent-linux)
        /// If platform is omitted (just a file path), it will be detected from headers.
        #[arg(long = "bin")]
        bins: Vec<String>,
        /// For single-binary upgrades (legacy): path to binary
        #[arg(long)]
        file: Option<String>,
        /// For single-binary upgrades (legacy): explicit platform
        #[arg(long = "platform")]
        target_platform: Option<String>,
        /// Publish all provided binaries (each to its matching platform)
        #[arg(long = "all-platforms", default_value_t = false)]
        all_platforms: bool,
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
    /// Deploy a .realm package locally (installs and starts component)
    DeployPackage {
        /// Path to .realm file
        #[arg(long)]
        file: String,
        /// Optional name override
        #[arg(long)]
        name: Option<String>,
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
    /// Package-related commands
    #[command(subcommand)]
    Package(PackageCommands),
    /// Job orchestration commands
    #[command(subcommand)]
    Job(JobCommands),
    /// Start management web interface
    Manage {
        /// Authentication method (for now, always authenticates)
        #[arg(long)]
        owner_key: bool,
        /// Session timeout in minutes
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },
    /// Show current runtime policy (native/QEMU)
    PolicyShow,
    /// Set runtime policy flags
    PolicySet {
        /// Allow native execution (true/false)
        #[arg(long)]
        native: Option<bool>,
        /// Allow QEMU emulation (true/false)
        #[arg(long)]
        qemu: Option<bool>,
    },
    /// List stored blobs (CAS)
    StorageLs,
    /// Pin or unpin a blob
    StoragePin { digest: String, #[arg(long)] pinned: bool },
    /// Garbage collect storage to target total size (bytes)
    StorageGc { target_total_bytes: u64 },
}

#[derive(Debug, Subcommand)]
enum PackageCommands {
    /// Create a .realm bundle from a directory
    Create {
        /// Directory containing component.wasm and optional static/config/seed-data
        #[arg(long, default_value = ".")]
        dir: String,
        /// Override component name used in manifest
        #[arg(long)]
        name: Option<String>,
        /// Output file path (.realm)
        #[arg(long)]
        output: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum JobCommands {
    /// Submit a job from a TOML specification
    Submit {
        /// Path to job TOML file
        file: String,
        /// Attach a local file as an asset (repeatable). Format: name=path or path
        #[arg(long = "asset")]
        assets: Vec<String>,
        /// Reuse an artifact from a completed job as input. Format: jobId:name (repeatable)
        #[arg(long = "use-artifact")]
        use_artifacts: Vec<String>,
    },
    /// List all jobs (running, scheduled, completed)
    List {
        /// Show only jobs with this status (pending, running, completed, failed)
        #[arg(long)]
        status: Option<String>,
        /// Maximum number of jobs to show
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Refresh job state from peers before listing
        #[arg(long)]
        fresh: bool,
    },
    /// List jobs as JSON (for scripts)
    ListJson {
        /// Show only jobs with this status (pending, running, completed, failed)
        #[arg(long)]
        status: Option<String>,
        /// Maximum number of jobs to show
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Query network for jobs and print JSON (first response)
    NetListJson {
        /// Show only jobs with this status (pending, running, completed, failed)
        #[arg(long)]
        status: Option<String>,
        /// Maximum number of jobs to show
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Show detailed status of a specific job
    Status {
        /// Job ID or name to query
        job_id: String,
    },
    /// Show detailed status of a specific job (JSON)
    StatusJson {
        /// Job ID to query (must be full ID)
        job_id: String,
    },
    /// Query a job over the network and print JSON
    NetStatusJson {
        /// Job ID to query
        job_id: String,
    },
    /// Cancel a running or scheduled job
    Cancel {
        /// Job ID or name to cancel
        job_id: String,
    },
    /// Show logs for a specific job
    Logs {
        /// Job ID or name to show logs for
        job_id: String,
        /// Number of recent log lines to show
        #[arg(long, default_value_t = 100)]
        tail: usize,
        /// Follow log output in real-time
        #[arg(long, short = 'f')]
        follow: bool,
    },
    /// List artifacts for a specific job
    Artifacts {
        /// Job ID to list artifacts for
        job_id: String,
    },
    /// Download an artifact from a specific job
    Download {
        /// Job ID
        job_id: String,
        /// Artifact name
        artifact_name: String,
        /// Output file path (optional, defaults to artifact name)
        #[arg(long, short = 'o')]
        output: Option<String>,
    },
    /// Print artifacts for a job in JSON for programmatic use
    ArtifactsJson { job_id: String },
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();

    // Default behavior: if no subcommand specified, run the agent
    let command = cli.command;

    match command {
        None => {
            let shutdown = setup_shutdown_handler();
            tokio::select! {
                result = p2p::run_agent(cli.wasm, cli.memory_max_mb, cli.fuel, cli.epoch_ms, cli.roles, false, None) => result,
                _ = shutdown => {
                    info!("Shutdown signal received, stopping agent gracefully");
                    Ok(())
                }
            }
        },
        Some(Commands::Init) => cmd::init().await,
        Some(Commands::KeyShow) => cmd::key_show().await,
        Some(Commands::Apply { wasm, file, version }) => cmd::apply(wasm, file, version).await,
        Some(Commands::Status) => cmd::status().await,
        #[cfg(unix)]
        Some(Commands::Install { binary, system }) => cmd::install(binary, system).await,
        #[cfg(not(unix))]
        Some(Commands::Install { .. }) => Err(anyhow::anyhow!("install is only supported on Unix-like systems with systemd")),
        Some(Commands::Upgrade { bins, file, target_platform, all_platforms, version, target_peers, target_tags }) =>
            cmd::upgrade_multi(bins, file, target_platform, all_platforms, version, target_peers, target_tags).await,
        Some(Commands::Invite { bootstrap, realm_id, exp_mins }) => cmd::invite(bootstrap, realm_id, exp_mins).await,
        Some(Commands::Enroll { token, binary, system }) => cmd::enroll(token, binary, system).await,
        Some(Commands::Configure { owner, bootstrap }) => cmd::configure(owner, bootstrap).await,
        Some(Commands::DiagQuic { addr }) => cmd::diag_quic(addr).await,
        Some(Commands::Whoami) => cmd::whoami().await,
        Some(Commands::Push { name, file, replicas, memory_max_mb, fuel, epoch_ms, mounts, ports, routes_static, visibility, target_peers, target_tags, start }) => cmd::push(name, file, replicas, memory_max_mb, fuel, epoch_ms, mounts, ports, routes_static, visibility, target_peers, target_tags, start).await,
        Some(Commands::DeployComponent { path, package, profile, features, target_peers, target_tags, name, start }) => cmd::deploy_component(path, package, profile, features, target_peers, target_tags, name, start).await,
        Some(Commands::DeployPackage { file, name }) => cmd::push_package(file, name).await,
        Some(Commands::Package(pkg_cmd)) => match pkg_cmd {
            PackageCommands::Create { dir, name, output } => cmd::package_create(dir, name, output).await,
        },
        Some(Commands::Manage { owner_key, timeout }) => {
            use std::time::Duration;
            let timeout_duration = Duration::from_secs(timeout * 60);
            let wasm = cli.wasm.clone();
            let memory_max_mb = cli.memory_max_mb;
            let fuel = cli.fuel;
            let epoch_ms = cli.epoch_ms;
            // Ensure the ephemeral UI agent is tagged for clarity and gating
            let mut roles = cli.roles.clone();
            if !roles.iter().any(|r| r == "ui") {
                roles.push("ui".to_string());
            }
            // Spawn ephemeral agent and tie its lifetime to the manage session
            // Shared peer status map between ephemeral agent and web session
            let shared_status: std::sync::Arc<tokio::sync::Mutex<std::collections::BTreeMap<String, common::Status>>> =
                std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::BTreeMap::new()));
            let status_for_agent = shared_status.clone();
            let agent_handle = tokio::spawn(async move {
                if let Err(e) = p2p::run_agent(wasm, memory_max_mb, fuel, epoch_ms, roles, true, Some(status_for_agent)).await {
                    warn!(error=%e, "temporary agent exited");
                }
            });
            let res = web::start_management_session(owner_key, timeout_duration, Some(shared_status)).await;
            // On manage exit (timeout/CTRL-C), cancel the ephemeral agent task
            agent_handle.abort();
            let _ = agent_handle.await;
            res
        },
        Some(Commands::Job(job_cmd)) => match job_cmd {
            JobCommands::Submit { file, assets, use_artifacts } => cmd::submit_job(file, assets, use_artifacts).await,
            JobCommands::List { status, limit, fresh } => cmd::list_jobs(status, limit, fresh).await,
            JobCommands::ListJson { status, limit } => cmd::list_jobs_json(status, limit).await,
            JobCommands::NetListJson { status, limit } => cmd::net_list_jobs_json(status, limit).await,
            JobCommands::Status { job_id } => cmd::job_status(job_id).await,
            JobCommands::StatusJson { job_id } => cmd::job_status_json(job_id).await,
            JobCommands::NetStatusJson { job_id } => cmd::net_status_job_json(job_id).await,
            JobCommands::Cancel { job_id } => cmd::cancel_job(job_id).await,
            JobCommands::Logs { job_id, tail, follow } => cmd::job_logs(job_id, tail, follow).await,
            JobCommands::Artifacts { job_id } => cmd::job_artifacts(job_id).await,
            JobCommands::Download { job_id, artifact_name, output } => cmd::job_download(job_id, artifact_name, output).await,
            JobCommands::ArtifactsJson { job_id } => cmd::job_artifacts_json(job_id).await,
        },
        Some(Commands::PolicyShow) => cmd::policy_show().await,
        Some(Commands::PolicySet { native, qemu }) => cmd::policy_set(native, qemu).await,
        Some(Commands::StorageLs) => cmd::storage_ls().await,
        Some(Commands::StoragePin { digest, pinned }) => cmd::storage_pin(digest, pinned).await,
        Some(Commands::StorageGc { target_total_bytes }) => cmd::storage_gc(target_total_bytes).await,
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
