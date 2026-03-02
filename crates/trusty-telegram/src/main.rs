//! trusty-telegram — Telegram bot interface for trusty-izzie.
//!
//! # Usage
//!
//! Pair a bot token (one-time setup):
//!   trusty-telegram pair --token <BOT_TOKEN> [--allowed-users 123456,789012]
//!
//! Start the bot:
//!   trusty-telegram start
//!   TELEGRAM_BOT_TOKEN=<TOKEN> trusty-telegram start

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use teloxide::prelude::*;
use teloxide::types::{ChatAction, ParseMode};
use tracing::{error, info};

use trusty_chat::{engine::ChatEngine, session::SessionManager};
use trusty_store::sqlite::SqliteStore;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

/// Telegram bot interface for trusty-izzie.
#[derive(Parser)]
#[command(
    name = "trusty-telegram",
    about = "Telegram bot for trusty-izzie",
    version
)]
struct Cli {
    /// Path to a custom configuration file.
    #[arg(long, global = true)]
    config: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Pair a Telegram bot token and configure allowed users.
    Pair {
        /// The bot token from @BotFather.
        #[arg(long)]
        token: String,
        /// Comma-separated list of allowed Telegram user IDs (optional).
        #[arg(long)]
        allowed_users: Option<String>,
    },
    /// Start the bot (default if no subcommand given).
    Start,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Expand a leading `~` to the value of `$HOME`.
fn expand_tilde(path: &str) -> PathBuf {
    if path.starts_with('~') {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(path.replacen('~', &home, 1))
    } else {
        PathBuf::from(path)
    }
}

// ---------------------------------------------------------------------------
// Bot handler
// ---------------------------------------------------------------------------

async fn run_bot(bot_token: String, engine: Arc<ChatEngine>, allowed_users: Vec<i64>) {
    info!("Starting Telegram bot long-polling");
    let bot = Bot::new(bot_token);

    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let engine = engine.clone();
        let allowed = allowed_users.clone();
        async move {
            // Authorisation check
            if !allowed.is_empty() {
                let uid = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
                if !allowed.contains(&uid) {
                    bot.send_message(msg.chat.id, "Not authorized.").await?;
                    return Ok(());
                }
            }

            let text = match msg.text() {
                Some(t) => t.to_string(),
                None => return Ok(()),
            };

            // Show typing indicator
            bot.send_chat_action(msg.chat.id, ChatAction::Typing)
                .await?;

            // Create a fresh session keyed to the Telegram chat ID.
            // SessionManager::load is not yet implemented, so we always start fresh.
            let session_key = format!("tg_{}", msg.chat.id);
            let mut session = SessionManager::new_session(&session_key);

            match engine.chat(&mut session, &text).await {
                Ok(response) => {
                    info!(
                        memories = response.memories_to_save.len(),
                        "Chat turn complete"
                    );
                    bot.send_message(msg.chat.id, &response.reply)
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Err(e) => {
                    error!("Chat error: {e}");
                    bot.send_message(msg.chat.id, "Sorry, I encountered an error.")
                        .await?;
                }
            }

            Ok(())
        }
    })
    .await;
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    trusty_core::init_logging("info");

    let config = trusty_core::load_config(cli.config.as_deref()).await?;

    // Open SQLite for config KV access (lightweight — no LanceDB or Kuzu needed here).
    let data_dir = expand_tilde(&config.storage.data_dir);
    let sqlite_path = data_dir.join(&config.storage.sqlite_path);
    std::fs::create_dir_all(&data_dir)?;
    let sqlite = Arc::new(SqliteStore::open(&sqlite_path)?);

    match cli.command.unwrap_or(Command::Start) {
        Command::Pair {
            token,
            allowed_users,
        } => {
            sqlite.set_config("telegram_bot_token", &token)?;
            if let Some(users) = allowed_users {
                sqlite.set_config("telegram_allowed_users", &users)?;
            }
            println!("Telegram bot token stored.");
            println!("Run 'trusty-telegram start' to launch the bot.");
        }

        Command::Start => {
            let token = std::env::var("TELEGRAM_BOT_TOKEN")
                .ok()
                .or_else(|| sqlite.get_config("telegram_bot_token").ok().flatten())
                .ok_or_else(|| {
                    anyhow!("No bot token found. Run: trusty-telegram pair --token <TOKEN>")
                })?;

            let allowed: Vec<i64> = sqlite
                .get_config("telegram_allowed_users")?
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .filter_map(|s| s.trim().parse().ok())
                .collect();

            let api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();

            let engine = Arc::new(ChatEngine::new(
                config.openrouter.base_url.clone(),
                api_key,
                config.openrouter.chat_model.clone(),
                config.chat.max_tool_iterations,
            ));

            if allowed.is_empty() {
                println!("trusty-telegram starting (no user restriction)...");
            } else {
                println!("trusty-telegram starting (allowed users: {:?})...", allowed);
            }

            run_bot(token, engine, allowed).await;
        }
    }

    Ok(())
}
