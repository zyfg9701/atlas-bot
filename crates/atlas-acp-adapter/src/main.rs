//! atlas-acp-adapter — α minimal ACP JSON-RPC over WS → Hub bot_client.
//!
//! Listens on ATLAS_ACP_BIND (default 127.0.0.1:8790). Connects inward to
//! ATLAS_HUB_WS (default ws://127.0.0.1:7700/ws). See docs/P6-alpha-runbook.md.

use std::net::SocketAddr;
use std::process::ExitCode;

use atlas_acp_adapter::{serve, DEFAULT_ACP_BIND, DEFAULT_HUB_WS, FROZEN_WHITELIST};
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "atlas-acp-adapter",
    about = "α ACP subset adapter (:8790) → Hub bot.* — not full ACP; α ≠ β′",
    version
)]
struct Args {
    /// ACP listen address (env ATLAS_ACP_BIND).
    #[arg(long, env = "ATLAS_ACP_BIND", default_value = DEFAULT_ACP_BIND)]
    bind: String,

    /// Hub WebSocket URL for inward bot_client (env ATLAS_HUB_WS).
    #[arg(long, env = "ATLAS_HUB_WS", default_value = DEFAULT_HUB_WS)]
    hub_ws: String,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    let args = Args::parse();
    let bind: SocketAddr = match args.bind.parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("invalid ATLAS_ACP_BIND / --bind: {e}");
            return ExitCode::FAILURE;
        }
    };

    info!(
        %bind,
        hub_ws = %args.hub_ws,
        whitelist = ?FROZEN_WHITELIST,
        "starting atlas-acp-adapter (α subset)"
    );

    match serve(bind, args.hub_ws).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("adapter exited: {e}");
            ExitCode::FAILURE
        }
    }
}
