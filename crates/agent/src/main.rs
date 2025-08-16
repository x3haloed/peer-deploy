mod p2p;
mod runner;
mod supervisor;

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
struct Args {
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
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();

    let args = Args::parse();

    p2p::run_agent(args.wasm, args.memory_max_mb, args.fuel, args.epoch_ms).await
}
