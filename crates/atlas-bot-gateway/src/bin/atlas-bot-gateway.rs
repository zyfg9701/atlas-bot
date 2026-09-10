//! Standalone gateway binary (P3.5 scheme B + R1 box sidecar).
//!
//! Listens on loopback by default (`127.0.0.1:8787`) and exposes:
//! - `POST /invoke` — Bot-Relay hot commands
//! - `GET  /healthz` — JSON `{ ok, backend, agent_cli?, agent_cli_found? }`
//! - `GET  /stats` — `{ invoke_count, backend, agent_cli?, agent_cli_found? }`
//!
//! Env:
//! - `ATLAS_GATEWAY_HTTP_BIND` — default `127.0.0.1:8787`
//! - `ATLAS_GATEWAY_BACKEND` — `cli` (default) | `openai` | `stub` | `box`
//! - `ATLAS_AGENT_CLI` — CLI binary for scheme B (default `agent`)
//! - `ATLAS_AGENT_CLI_EXTRA_ARGS` — JSON string array
//! - `ATLAS_OPENAI_*` — when backend=`openai`
//! - `ATLAS_BOX_WORKSPACE` / `ATLAS_BOX_TURN_DELAY_MS` — when backend=`box`
//! - `ATLAS_HUB_EVENT_URL` / `ATLAS_HUB_EVENT_TOKEN` — B1 mid-turn RuntimeHint POST
//!   to Hub loopback ingest (see docs/b1-event-ingest-runbook.md)
//!
//! Primary path: see `docs/cli-primary-runbook.md`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use atlas_bot_gateway::{
    resolve_agent_cli_found, serve_gateway_http_with_meta, BoxSidecarGateway, CliAgentGateway,
    Gateway, GatewayHttpMeta, InMemoryGateway, OpenAiCompatGateway, ENV_AGENT_CLI,
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

    let (gw, meta): (Arc<dyn Gateway>, GatewayHttpMeta) = match backend.as_str() {
        "stub" | "memory" | "inmemory" => {
            info!("backend=stub (InMemoryGateway echo)");
            (
                Arc::new(InMemoryGateway::new()),
                GatewayHttpMeta::for_backend("stub"),
            )
        }
        "openai" => {
            info!("backend=openai (fallback only)");
            let g = OpenAiCompatGateway::from_env().expect("openai env");
            (
                Arc::new(g),
                GatewayHttpMeta::for_backend("openai"),
            )
        }
        "box" | "sidecar" | "box-sidecar" => {
            info!("backend=box (BoxSidecarGateway R1)");
            (
                Arc::new(BoxSidecarGateway::from_env()),
                GatewayHttpMeta::for_backend("box"),
            )
        }
        "cli" | _ => {
            let cli_raw = std::env::var(ENV_AGENT_CLI).unwrap_or_else(|_| "agent".into());
            let cli_path = PathBuf::from(&cli_raw);
            let found = resolve_agent_cli_found(&cli_path);
            info!(
                cli = %cli_raw,
                agent_cli_found = found,
                "backend=cli (Cursor/Atlas Agent CLI adapter)"
            );
            if !found {
                tracing::warn!(
                    cli = %cli_raw,
                    "ATLAS_AGENT_CLI not found on PATH/disk — sendPrompt will fail visibly (not stub echo). See docs/cli-primary-runbook.md"
                );
            }
            (
                Arc::new(CliAgentGateway::from_env()),
                GatewayHttpMeta::for_backend("cli").with_cli(cli_raw, found),
            )
        }
    };

    info!(%bind, %backend, "atlas-bot-gateway starting");
    serve_gateway_http_with_meta(gw, bind, meta)
        .await
        .expect("serve");
}
