//! Standalone real gateway (P3.5 scheme B).
//!
//! Listens on loopback by default (`127.0.0.1:8787`) and exposes:
//! - `POST /invoke` — Bot-Relay hot commands
//! - `GET  /healthz`
//! - `GET  /stats` — `{ invoke_count }`
//!
//! Env:
//! - `ATLAS_GATEWAY_HTTP_BIND` — default `127.0.0.1:8787`
//! - `ATLAS_GATEWAY_BACKEND` — `cli` (default) | `openai` | `stub`
//! - `ATLAS_AGENT_CLI` — CLI binary for scheme B (default `agent`)
//! - `ATLAS_AGENT_CLI_EXTRA_ARGS` — JSON string array
//! - `ATLAS_OPENAI_*` — when backend=`openai`

use std::net::SocketAddr;
use std::sync::Arc;

use atlas_bot_gateway::{
    serve_gateway_http, CliAgentGateway, Gateway, InMemoryGateway, OpenAiCompatGateway,
};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    let bind: SocketAddr = std::env::var("ATLAS_GATEWAY_HTTP_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8787".into())
        .parse()
        .expect("ATLAS_GATEWAY_HTTP_BIND");

    let backend = std::env::var("ATLAS_GATEWAY_BACKEND")
        .unwrap_or_else(|_| "cli".into())
        .to_ascii_lowercase();

    let gw: Arc<dyn Gateway> = match backend.as_str() {
        "stub" | "memory" | "inmemory" => {
            info!("backend=stub (InMemoryGateway echo)");
            Arc::new(InMemoryGateway::new())
        }
        "openai" => {
            info!("backend=openai (fallback only)");
            let g = OpenAiCompatGateway::from_env().expect("openai env");
            Arc::new(g)
        }
        "cli" | _ => {
            info!("backend=cli (Cursor/Atlas Agent CLI adapter)");
            Arc::new(CliAgentGateway::from_env())
        }
    };

    info!(%bind, %backend, "atlas-bot-gateway starting");
    serve_gateway_http(gw, bind).await.expect("serve");
}
