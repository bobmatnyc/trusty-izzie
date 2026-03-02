//! trusty-telegram — Telegram bot interface for trusty-izzie.
//!
//! # Usage
//!
//! Pair a bot token (one-time setup):
//!   trusty-telegram pair --token <BOT_TOKEN> [--allowed-users 123456,789012]
//!
//! Start in webhook mode (default):
//!   trusty-telegram start --webhook-url https://izzie.ngrok.dev/webhook/telegram
//!
//! Start in long-poll fallback mode:
//!   trusty-telegram start --poll
//!
//! Manage webhook registration:
//!   trusty-telegram webhook set --url https://izzie.ngrok.dev/webhook/telegram
//!   trusty-telegram webhook clear

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use axum::extract::{Json, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Router;
use clap::{Parser, Subcommand};
use serde::Deserialize;
use teloxide::prelude::*;
use teloxide::types::ChatAction;
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
    Start {
        /// Run in webhook mode at this public URL (default mode).
        #[arg(long)]
        webhook_url: Option<String>,
        /// Port to bind the axum webhook server on (default: 3457).
        #[arg(long, default_value = "3457")]
        port: u16,
        /// Fall back to long-polling mode instead of webhook.
        #[arg(long)]
        poll: bool,
    },
    /// Manage Telegram webhook registration.
    Webhook {
        #[command(subcommand)]
        action: WebhookAction,
    },
}

#[derive(Subcommand)]
enum WebhookAction {
    /// Register a webhook URL with Telegram.
    Set {
        /// The public HTTPS URL Telegram should POST updates to.
        #[arg(long)]
        url: String,
    },
    /// Remove the currently registered webhook.
    Clear,
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

/// Call the Telegram Bot API `setWebhook` endpoint.
async fn api_set_webhook(token: &str, url: &str) -> Result<()> {
    let endpoint = format!("https://api.telegram.org/bot{token}/setWebhook");
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .post(&endpoint)
        .json(&serde_json::json!({ "url": url }))
        .send()
        .await?
        .json()
        .await?;
    if resp["ok"].as_bool().unwrap_or(false) {
        info!("Webhook registered: {url}");
        println!("Webhook set to: {url}");
    } else {
        let desc = resp["description"].as_str().unwrap_or("unknown error");
        return Err(anyhow!("setWebhook failed: {desc}"));
    }
    Ok(())
}

/// Call the Telegram Bot API `deleteWebhook` endpoint.
async fn api_delete_webhook(token: &str) -> Result<()> {
    let endpoint = format!("https://api.telegram.org/bot{token}/deleteWebhook");
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client.post(&endpoint).send().await?.json().await?;
    if resp["ok"].as_bool().unwrap_or(false) {
        info!("Webhook cleared");
        println!("Webhook cleared.");
    } else {
        let desc = resp["description"].as_str().unwrap_or("unknown error");
        return Err(anyhow!("deleteWebhook failed: {desc}"));
    }
    Ok(())
}

/// Send a plain-text reply via the Telegram Bot API directly (no teloxide HTML issues).
async fn send_reply(token: &str, chat_id: i64, text: &str) -> Result<()> {
    let endpoint = format!("https://api.telegram.org/bot{token}/sendMessage");
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .post(&endpoint)
        .json(&serde_json::json!({
            "chat_id": chat_id,
            "text": text,
        }))
        .send()
        .await?
        .json()
        .await?;
    if !resp["ok"].as_bool().unwrap_or(false) {
        let desc = resp["description"].as_str().unwrap_or("unknown error");
        error!("sendMessage failed: {desc}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Webhook mode — axum server
// ---------------------------------------------------------------------------

/// Minimal Update struct to avoid pulling in all of teloxide types for parsing.
#[derive(Deserialize)]
struct IncomingUpdate {
    message: Option<IncomingMessage>,
}

#[derive(Deserialize)]
struct IncomingMessage {
    chat: IncomingChat,
    from: Option<IncomingUser>,
    text: Option<String>,
}

#[derive(Deserialize)]
struct IncomingChat {
    id: i64,
}

#[derive(Deserialize)]
struct IncomingUser {
    id: i64,
}

/// Shared state for the axum webhook handler.
struct WebhookState {
    engine: Arc<ChatEngine>,
    allowed_users: Vec<i64>,
    bot_token: String,
}

async fn health_handler() -> StatusCode {
    StatusCode::OK
}

async fn webhook_handler(
    State(state): State<Arc<WebhookState>>,
    Json(update): Json<IncomingUpdate>,
) -> StatusCode {
    let msg = match update.message {
        Some(m) => m,
        None => return StatusCode::OK,
    };

    let text = match msg.text {
        Some(t) => t,
        None => return StatusCode::OK,
    };

    let chat_id = msg.chat.id;

    // Authorisation check
    if !state.allowed_users.is_empty() {
        let uid = msg.from.map(|u| u.id).unwrap_or(0);
        if !state.allowed_users.contains(&uid) {
            let token = state.bot_token.clone();
            tokio::spawn(async move {
                let _ = send_reply(&token, chat_id, "Not authorized.").await;
            });
            return StatusCode::OK;
        }
    }

    // Process chat asynchronously — respond 200 immediately to Telegram.
    let engine = state.engine.clone();
    let token = state.bot_token.clone();
    tokio::spawn(async move {
        let session_key = format!("tg_{chat_id}");
        let mut session = SessionManager::new_session(&session_key);

        match engine.chat(&mut session, &text).await {
            Ok(response) => {
                info!(
                    memories = response.memories_to_save.len(),
                    "Chat turn complete"
                );
                let _ = send_reply(&token, chat_id, &response.reply).await;
            }
            Err(e) => {
                error!("Chat error: {e}");
                let _ = send_reply(&token, chat_id, "Sorry, I encountered an error.").await;
            }
        }
    });

    StatusCode::OK
}

async fn run_webhook(
    bot_token: String,
    webhook_url: String,
    port: u16,
    engine: Arc<ChatEngine>,
    allowed_users: Vec<i64>,
) -> Result<()> {
    // Register webhook with Telegram.
    api_set_webhook(&bot_token, &webhook_url).await?;

    let state = Arc::new(WebhookState {
        engine,
        allowed_users,
        bot_token,
    });

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/webhook/telegram", post(webhook_handler))
        .with_state(state);

    let addr = format!("0.0.0.0:{port}");
    info!("Webhook server listening on {addr}");
    println!("trusty-telegram webhook server on port {port}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Long-poll mode (fallback)
// ---------------------------------------------------------------------------

async fn run_poll(bot_token: String, engine: Arc<ChatEngine>, allowed_users: Vec<i64>) {
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

            let session_key = format!("tg_{}", msg.chat.id);
            let mut session = SessionManager::new_session(&session_key);

            match engine.chat(&mut session, &text).await {
                Ok(response) => {
                    info!(
                        memories = response.memories_to_save.len(),
                        "Chat turn complete"
                    );
                    bot.send_message(msg.chat.id, &response.reply).await?;
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

    // Open SQLite for config KV access.
    let data_dir = expand_tilde(&config.storage.data_dir);
    let sqlite_path = data_dir.join(&config.storage.sqlite_path);
    std::fs::create_dir_all(&data_dir)?;
    let sqlite = Arc::new(SqliteStore::open(&sqlite_path)?);

    let default_start = Command::Start {
        webhook_url: None,
        port: 3457,
        poll: false,
    };

    match cli.command.unwrap_or(default_start) {
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

        Command::Start {
            webhook_url,
            port,
            poll,
        } => {
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

            if poll {
                run_poll(token, engine, allowed).await;
            } else {
                // Webhook mode — URL required.
                let url = webhook_url.ok_or_else(|| {
                    anyhow!(
                        "Webhook URL required. Use --webhook-url <URL> or --poll for long-polling."
                    )
                })?;
                run_webhook(token, url, port, engine, allowed).await?;
            }
        }

        Command::Webhook { action } => {
            let token = std::env::var("TELEGRAM_BOT_TOKEN")
                .ok()
                .or_else(|| sqlite.get_config("telegram_bot_token").ok().flatten())
                .ok_or_else(|| {
                    anyhow!("No bot token found. Run: trusty-telegram pair --token <TOKEN>")
                })?;

            match action {
                WebhookAction::Set { url } => {
                    api_set_webhook(&token, &url).await?;
                }
                WebhookAction::Clear => {
                    api_delete_webhook(&token).await?;
                }
            }
        }
    }

    Ok(())
}
