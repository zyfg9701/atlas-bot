//! atlas-bot-cli — P6 β′ Bot-Relay thin client (not ACP / not official atlas).

use std::process::ExitCode;

use atlas_bot_cli::{CliError, HubClient, DEFAULT_AGENT_ID, DEFAULT_HUB_WS};
use clap::{Parser, Subcommand};
use serde_json::Value;

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

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
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
    let client = HubClient::connect(&args.url).await?;
    client.assert_no_acp_capabilities()?;

    match args.cmd {
        Cmd::Status => {
            let ack = client.hello_ack();
            println!(
                "hello_ack connection_id={} hub={} caps={}",
                ack.get("connection_id").and_then(|v| v.as_str()).unwrap_or("?"),
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
