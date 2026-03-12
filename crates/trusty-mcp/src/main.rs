//! trusty-mcp — MCP server exposing Izzie's personal data to AI clients.
//!
//! Usage:
//!   trusty-mcp --stdio         # Claude Desktop / Cursor stdio mode
//!   trusty-mcp --port 3458     # HTTP SSE mode (default)

mod protocol;
mod server;
mod tools;
mod transport;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};
use trusty_chat::context::ContextAssembler;
use trusty_chat::engine::ChatEngine;
use trusty_core::load_config;
use trusty_metro_north::MetroNorthSkill;
use trusty_store::SqliteStore;
use trusty_weather::WeatherSkill;

use server::McpServer;

#[derive(Parser)]
#[command(name = "trusty-mcp", about = "Izzie MCP server")]
struct Args {
    /// Run as a stdio MCP server (for Claude Desktop / Cursor).
    #[arg(long)]
    stdio: bool,

    /// HTTP port for SSE transport.
    #[arg(long, default_value = "3458")]
    port: u16,
}

fn expand_tilde(raw: &str) -> PathBuf {
    if raw.starts_with('~') {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(raw.replacen('~', &home, 1))
    } else {
        PathBuf::from(raw)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Always write logs to stderr — in stdio mode stdout is the JSON-RPC stream.
    let log_level = std::env::var("TRUSTY_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    let filter = EnvFilter::try_new(&log_level).unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt::Subscriber::builder()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_thread_ids(false)
        .try_init();

    let config = load_config(None).await?;

    let data_dir = expand_tilde(&config.storage.data_dir);
    let sqlite_path = data_dir.join(&config.storage.sqlite_path);
    let sqlite = Arc::new(SqliteStore::open(&sqlite_path)?);

    let api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
    let assembler = ContextAssembler::new(
        config.chat.context_memory_limit,
        config.chat.context_entity_limit,
    );
    let agents_dir = expand_tilde(&config.agents.agents_dir);
    let skills_dir = expand_tilde(&config.agents.skills_dir);

    let skills: Vec<Arc<dyn trusty_skill::Skill>> =
        vec![Arc::new(MetroNorthSkill), Arc::new(WeatherSkill)];
    let engine = Arc::new(
        ChatEngine::new_with_context(
            config.openrouter.base_url.clone(),
            api_key,
            config.openrouter.chat_model.clone(),
            config.chat.max_tool_iterations,
            assembler,
        )
        .with_sqlite(Arc::clone(&sqlite))
        .with_agents_dir(agents_dir)
        .with_skills_dir(skills_dir.to_string_lossy().into_owned())
        .with_skills(skills),
    );

    let server = Arc::new(McpServer::new(engine));

    if args.stdio {
        info!("starting trusty-mcp in stdio mode");
        transport::stdio::run_stdio(server).await
    } else {
        transport::sse::run_http(server, args.port).await
    }
}
