//! atlas-bot-cli — P6 β′ Bot-Relay thin client + I2.1 / W1 login/logout.

use std::process::ExitCode;
use std::time::Duration;

use atlas_bot_auth_client::mock_oidc::drive_authorize_for_code;
use atlas_bot_auth_client::mock_wecom::drive_wecom_authorize_for_code;
use atlas_bot_auth_client::{
    build_authorize_request, build_wecom_authorize_request, delete_credentials, exchange_code,
    exchange_wecom_code, load_bearer, peek_sub, pick_hub_bearer, redirect_port_from_env,
    save_credentials, start_loopback, wait_for_callback, OidcClientConfig, PkcePair,
    StoredCredentials, TicketProvider, WeComClientConfig,
};
use atlas_bot_cli::{CliError, HubClient, DEFAULT_AGENT_ID, DEFAULT_HUB_WS};
use clap::{Parser, Subcommand};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(
    name = "atlas-bot-cli",
    about = "Thin Bot-Relay CLI (bot_client → Hub WS). Bot-Relay ≠ ACP; not 100% official atlas compatible.",
    version
)]
struct Args {
    /// Hub WebSocket URL (env ATLAS_HUB_WS overrides default).
    #[arg(
        long,
        short = 'u',
        env = "ATLAS_HUB_WS",
        default_value = DEFAULT_HUB_WS,
        global = true
    )]
    url: String,

    /// Default agent id for prompt / list --hot / interrupt.
    #[arg(long, short = 'a', default_value = DEFAULT_AGENT_ID, global = true)]
    agent: String,

    /// Optional Bearer override (else load ~/.config/atlas-bot/credentials.json).
    #[arg(long, env = "ATLAS_HUB_BEARER", global = true)]
    bearer: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Login (OIDC PKCE or WeCom → Hub exchange). Default provider: oidc.
    Login {
        /// Ticket provider: `oidc` (default) or `wecom`.
        #[arg(long, env = "ATLAS_TICKET_PROVIDER", default_value = "oidc")]
        provider: String,
        /// OIDC issuer base URL (…/authorize + …/token derived). Required for oidc.
        #[arg(long, env = "ATLAS_OIDC_ISSUER")]
        issuer: Option<String>,
        /// OIDC client_id.
        #[arg(long, env = "ATLAS_OIDC_CLIENT_ID", default_value = "atlas-bot-cli")]
        client_id: String,
        /// Optional audience hint.
        #[arg(long, env = "ATLAS_OIDC_AUDIENCE")]
        audience: Option<String>,
        /// WeCom corp id (or ATLAS_WECOM_CORP_ID).
        #[arg(long, env = "ATLAS_WECOM_CORP_ID")]
        wecom_corp_id: Option<String>,
        /// WeCom agent id.
        #[arg(long, env = "ATLAS_WECOM_AGENT_ID")]
        wecom_agent_id: Option<String>,
        /// WeCom / mock authorize base (ATLAS_WECOM_AUTHORIZE_BASE or ATLAS_WECOM_API_BASE).
        #[arg(long, env = "ATLAS_WECOM_AUTHORIZE_BASE")]
        wecom_authorize_base: Option<String>,
        /// Hub HTTP base for WeCom exchange (default: derive from --url / ATLAS_HUB_HTTP).
        #[arg(long, env = "ATLAS_HUB_HTTP")]
        hub_http: Option<String>,
    },
    /// Delete local credentials.
    Logout,
    /// hello + cold bot.status (does not use bot.command).
    Status,
    /// Cold bot.roster; optional --hot also calls listAgents.
    List {
        /// Also invoke hot listAgents via bot.command.
        #[arg(long)]
        hot: bool,
    },
    /// subscribe → sendPrompt → wait hub:turn_finished → transcriptTail.
    Prompt {
        /// Prompt text.
        text: String,
    },
    /// Alias for prompt.
    Chat {
        text: String,
    },
    /// interruptAgentRun for the selected agent.
    Interrupt,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", e.display_line());
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<(), CliError> {
    match args.cmd {
        Cmd::Login {
            provider,
            issuer,
            client_id,
            audience,
            wecom_corp_id,
            wecom_agent_id,
            wecom_authorize_base,
            hub_http,
        } => {
            let prov = TicketProvider::parse(&provider).ok_or_else(|| {
                CliError::Message(format!(
                    "unknown --provider {provider:?} (expected oidc|wecom)"
                ))
            })?;
            match prov {
                TicketProvider::Oidc => {
                    let issuer = issuer.ok_or_else(|| {
                        CliError::Message(
                            "OIDC login requires --issuer / ATLAS_OIDC_ISSUER".into(),
                        )
                    })?;
                    run_login_oidc(&issuer, &client_id, audience.as_deref()).await?;
                }
                TicketProvider::WeCom => {
                    run_login_wecom(
                        wecom_corp_id.as_deref(),
                        wecom_agent_id.as_deref(),
                        wecom_authorize_base.as_deref(),
                        hub_http.as_deref(),
                        &args.url,
                    )
                    .await?;
                }
            }
            return Ok(());
        }
        Cmd::Logout => {
            let removed = delete_credentials().map_err(|e| CliError::Message(e.to_string()))?;
            if removed {
                println!("logged out (credentials deleted)");
            } else {
                println!("no credentials on disk");
            }
            return Ok(());
        }
        _ => {}
    }

    let bearer = resolve_bearer(args.bearer.as_deref())?;
    let client = HubClient::connect_with_bearer(&args.url, bearer.as_deref()).await?;
    client.assert_no_acp_capabilities()?;

    match args.cmd {
        Cmd::Login { .. } | Cmd::Logout => unreachable!(),
        Cmd::Status => {
            let ack = client.hello_ack();
            println!(
                "hello_ack connection_id={} user_id={} hub={} caps={}",
                ack.get("connection_id").and_then(|v| v.as_str()).unwrap_or("?"),
                ack.get("user_id").and_then(|v| v.as_str()).unwrap_or("?"),
                ack.get("computer_hub_version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?"),
                client.capabilities().join(",")
            );
            let st = client.status().await?;
            println!("{}", pretty(&st));
        }
        Cmd::List { hot } => {
            let roster = client.roster().await?;
            println!("roster:\n{}", pretty(&roster));
            if hot {
                let listed = client.list_agents(&args.agent).await?;
                println!("listAgents:\n{}", pretty(&listed));
            }
        }
        Cmd::Prompt { text } | Cmd::Chat { text } => {
            let (send, turn, tail) = client.prompt_closed_loop(&args.agent, &text).await?;
            println!("sendPrompt:\n{}", pretty(&send));
            println!("turn_finished:\n{}", pretty(&turn));
            println!("transcriptTail:\n{}", pretty(&tail));
            if let Some(preview) = turn
                .get("event")
                .and_then(|e| e.get("preview"))
                .and_then(|p| p.as_str())
            {
                println!("reply_preview: {preview}");
            }
            print_tail_texts(&tail);
        }
        Cmd::Interrupt => {
            let r = client.interrupt(&args.agent).await?;
            println!("{}", pretty(&r));
        }
    }
    Ok(())
}

fn resolve_bearer(cli_flag: Option<&str>) -> Result<Option<String>, CliError> {
    if let Some(b) = cli_flag {
        if !b.is_empty() {
            return Ok(Some(b.to_string()));
        }
    }
    load_bearer().map_err(|e| CliError::Message(e.to_string()))
}

async fn run_login_oidc(
    issuer: &str,
    client_id: &str,
    audience: Option<&str>,
) -> Result<(), CliError> {
    let mut cfg = OidcClientConfig::from_issuer(issuer, client_id);
    if let Some(aud) = audience {
        cfg = cfg.with_audience(aud);
    }

    let port = redirect_port_from_env();
    let (_addr, redirect_uri, wait) = start_loopback(port)
        .await
        .map_err(|e| CliError::Message(e.to_string()))?;

    let pkce = PkcePair::generate();
    let state = Uuid::new_v4().to_string();
    let auth_req = build_authorize_request(&cfg, &redirect_uri, pkce, &state)
        .map_err(|e| CliError::Message(e.to_string()))?;

    let no_browser = std::env::var("ATLAS_I2_NO_BROWSER")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    println!("authorize_url={}", auth_req.url);
    println!("redirect_uri={redirect_uri}");
    println!("provider=oidc");

    if no_browser {
        let (code, got_state) = drive_authorize_for_code(&auth_req.url)
            .await
            .map_err(|e| CliError::Message(e.to_string()))?;
        if let Some(st) = got_state {
            if st != state {
                return Err(CliError::Message("OIDC state mismatch".into()));
            }
        }
        drop(wait);
        finish_login_oidc(&cfg, &redirect_uri, &code, &auth_req.pkce.verifier).await?;
    } else {
        open_browser(&auth_req.url);
        println!("Waiting for browser callback on {redirect_uri} …");
        let cb = wait_for_callback(wait, Duration::from_secs(180))
            .await
            .map_err(|e| CliError::Message(e.to_string()))?;
        if let Some(st) = cb.state.as_deref() {
            if st != state {
                return Err(CliError::Message("OIDC state mismatch".into()));
            }
        }
        finish_login_oidc(&cfg, &redirect_uri, &cb.code, &auth_req.pkce.verifier).await?;
    }
    Ok(())
}

async fn finish_login_oidc(
    cfg: &OidcClientConfig,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> Result<(), CliError> {
    let tr = exchange_code(cfg, redirect_uri, code, verifier)
        .await
        .map_err(|e| CliError::Message(e.to_string()))?;
    let bearer = pick_hub_bearer(&tr)
        .ok_or_else(|| CliError::Message("no id_token/access_token in token response".into()))?;
    let subject = peek_sub(&bearer);
    let expires_at = tr.expires_in.map(|s| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        now + s
    });
    let stored = StoredCredentials {
        access_token: bearer,
        id_token: tr.id_token.clone(),
        refresh_token: tr.refresh_token.clone(),
        expires_at,
        token_type: tr.token_type.clone(),
        subject: subject.clone(),
        provider: Some("oidc".into()),
    };
    let path = save_credentials(&stored).map_err(|e| CliError::Message(e.to_string()))?;
    println!(
        "login ok provider=oidc subject={} credentials={}",
        subject.as_deref().unwrap_or("?"),
        path.display()
    );
    Ok(())
}

async fn run_login_wecom(
    corp_id: Option<&str>,
    agent_id: Option<&str>,
    authorize_base: Option<&str>,
    hub_http: Option<&str>,
    hub_ws: &str,
) -> Result<(), CliError> {
    let corp_id = corp_id
        .map(|s| s.to_string())
        .or_else(|| std::env::var("ATLAS_WECOM_CORP_ID").ok())
        .ok_or_else(|| CliError::Message("WeCom login requires ATLAS_WECOM_CORP_ID".into()))?;
    let agent_id = agent_id
        .map(|s| s.to_string())
        .or_else(|| std::env::var("ATLAS_WECOM_AGENT_ID").ok())
        .ok_or_else(|| CliError::Message("WeCom login requires ATLAS_WECOM_AGENT_ID".into()))?;
    let authorize_base = authorize_base
        .map(|s| s.to_string())
        .or_else(|| std::env::var("ATLAS_WECOM_AUTHORIZE_BASE").ok())
        .or_else(|| std::env::var("ATLAS_WECOM_API_BASE").ok())
        .unwrap_or_else(|| "https://open.weixin.qq.com/connect/oauth2".into());
    let hub_http_base = hub_http
        .map(|s| s.trim_end_matches('/').to_string())
        .or_else(|| std::env::var("ATLAS_HUB_HTTP").ok())
        .unwrap_or_else(|| atlas_bot_auth_client::ws_to_http_base(hub_ws));

    let cfg = WeComClientConfig {
        corp_id,
        agent_id,
        authorize_base: authorize_base.trim_end_matches('/').to_string(),
        hub_http_base,
    };

    let redirect_override = std::env::var("ATLAS_WECOM_REDIRECT_URI").ok();
    let port = redirect_port_from_env();
    let (_addr, loopback_uri, wait) = start_loopback(port)
        .await
        .map_err(|e| CliError::Message(e.to_string()))?;
    let redirect_uri = redirect_override.unwrap_or(loopback_uri);

    let state = Uuid::new_v4().to_string();
    let auth_req = build_wecom_authorize_request(&cfg, &redirect_uri, &state)
        .map_err(|e| CliError::Message(e.to_string()))?;

    let no_browser = std::env::var("ATLAS_I2_NO_BROWSER")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    println!("authorize_url={}", auth_req.url);
    println!("redirect_uri={redirect_uri}");
    println!("provider=wecom");
    println!("hub_exchange={}/auth/wecom/exchange", cfg.hub_http_base);

    let code = if no_browser {
        let (code, got_state) = drive_wecom_authorize_for_code(&auth_req.url)
            .await
            .map_err(|e| CliError::Message(e.to_string()))?;
        if let Some(st) = got_state {
            if st != state {
                return Err(CliError::Message("WeCom state mismatch".into()));
            }
        }
        drop(wait);
        code
    } else {
        open_browser(&auth_req.url);
        println!("Waiting for browser callback on {redirect_uri} …");
        let cb = wait_for_callback(wait, Duration::from_secs(180))
            .await
            .map_err(|e| CliError::Message(e.to_string()))?;
        if let Some(st) = cb.state.as_deref() {
            if st != state {
                return Err(CliError::Message("WeCom state mismatch".into()));
            }
        }
        cb.code
    };

    finish_login_wecom(&cfg.hub_http_base, &code, Some(&state)).await
}

async fn finish_login_wecom(
    hub_http_base: &str,
    code: &str,
    state: Option<&str>,
) -> Result<(), CliError> {
    let tr = exchange_wecom_code(hub_http_base, code, state)
        .await
        .map_err(|e| CliError::Message(e.to_string()))?;
    let subject = tr
        .subject
        .clone()
        .or_else(|| peek_sub(&tr.access_token));
    let expires_at = tr.expires_in.map(|s| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        now + s
    });
    let stored = StoredCredentials {
        access_token: tr.access_token,
        id_token: None,
        refresh_token: None,
        expires_at,
        token_type: tr.token_type.or(Some("Bearer".into())),
        subject: subject.clone(),
        provider: Some("wecom".into()),
    };
    let path = save_credentials(&stored).map_err(|e| CliError::Message(e.to_string()))?;
    println!(
        "login ok provider=wecom subject={} credentials={}",
        subject.as_deref().unwrap_or("?"),
        path.display()
    );
    Ok(())
}

fn open_browser(url: &str) {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

fn print_tail_texts(tail: &Value) {
    if let Some(entries) = tail.get("entries").and_then(|e| e.as_array()) {
        for e in entries {
            if let Some(t) = e.get("text").and_then(|t| t.as_str()) {
                let role = e.get("role").and_then(|r| r.as_str()).unwrap_or("?");
                println!("  [{role}] {t}");
            }
        }
    }
}
