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

mod channel;
mod email_sync;
mod er_persist;
mod gdrive;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use axum::extract::{Json, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Router;
use clap::{Parser, Subcommand};
use serde::Deserialize;
use teloxide::prelude::*;
use teloxide::types::ChatAction;
use tracing::{error, info, warn};

use trusty_chat::{context::ContextAssembler, engine::ChatEngine, session::SessionManager};
use trusty_email::auth::{generate_pkce_pair, GoogleAuthClient};
use trusty_extractor::{EntityExtractor, ExtractorConfig, UserContext};
use trusty_store::sqlite::SqliteStore;
use trusty_store::Store;

use er_persist::persist_extraction_result;
use gdrive::spawn_drive_enrichment;

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
    document: Option<IncomingDocument>,
    caption: Option<String>,
}

#[derive(Deserialize, Clone)]
struct IncomingDocument {
    file_id: String,
    file_name: Option<String>,
    mime_type: Option<String>,
}

#[derive(Deserialize)]
struct IncomingChat {
    id: i64,
}

#[derive(Deserialize)]
struct IncomingUser {
    id: i64,
}

/// Query parameters for the Google OAuth callback route.
#[derive(serde::Deserialize)]
struct OAuthCallbackQuery {
    code: Option<String>,
    error: Option<String>,
}

/// Shared state for the axum webhook handler.
struct WebhookState {
    engine: Arc<ChatEngine>,
    allowed_users: Vec<i64>,
    bot_token: String,
    sqlite: Arc<SqliteStore>,
    extractor: Arc<EntityExtractor>,
    store: Arc<Store>,
    user_context: UserContext,
    min_occurrences: u32,
    gdrive_token: Option<String>,
}

async fn health_handler() -> StatusCode {
    StatusCode::OK
}

async fn oauth_callback_handler(
    State(state): State<Arc<WebhookState>>,
    Query(params): Query<OAuthCallbackQuery>,
) -> (StatusCode, axum::response::Html<String>) {
    if let Some(err) = params.error {
        return (
            StatusCode::BAD_REQUEST,
            axum::response::Html(format!(
                "<html><body><h2>OAuth error: {err}</h2></body></html>"
            )),
        );
    }
    let code = match params.code {
        Some(c) => c,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                axum::response::Html(
                    "<html><body><h2>Missing code parameter.</h2></body></html>".to_string(),
                ),
            );
        }
    };

    let verifier = match state.sqlite.get_config("oauth_pkce_verifier") {
        Ok(Some(v)) if !v.is_empty() => v,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::response::Html(
                    "<html><body><h2>No PKCE verifier found. Run 'trusty auth' first.</h2></body></html>"
                        .to_string(),
                ),
            );
        }
    };

    let client_id = std::env::var("GOOGLE_CLIENT_ID").unwrap_or_default();
    let client_secret = std::env::var("GOOGLE_CLIENT_SECRET").unwrap_or_default();

    let client = reqwest::Client::new();
    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("code", code.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            (
                "redirect_uri",
                "https://izzie.ngrok.dev/api/auth/google/callback",
            ),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            error!("Token exchange request failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::response::Html(format!(
                    "<html><body><h2>Token exchange failed: {e}</h2></body></html>"
                )),
            );
        }
    };

    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            error!("Token response parse failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::response::Html(format!(
                    "<html><body><h2>Failed to parse token response: {e}</h2></body></html>"
                )),
            );
        }
    };

    let access_token = match body["access_token"].as_str() {
        Some(t) => t.to_string(),
        None => {
            let msg = body
                .get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            error!("Token exchange error: {msg}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::response::Html(format!(
                    "<html><body><h2>Token exchange error: {msg}</h2></body></html>"
                )),
            );
        }
    };
    let refresh_token = body["refresh_token"].as_str().unwrap_or("").to_string();

    if let Err(e) = state
        .sqlite
        .set_config("google_access_token", &access_token)
    {
        error!("Failed to store access token: {e}");
    }
    if !refresh_token.is_empty() {
        if let Err(e) = state
            .sqlite
            .set_config("google_refresh_token", &refresh_token)
        {
            error!("Failed to store refresh token: {e}");
        }
    }
    // Clear the one-time verifier.
    let _ = state.sqlite.set_config("oauth_pkce_verifier", "");

    // Notify the user in Telegram if we have a pending chat_id.
    if let Ok(Some(s)) = state.sqlite.get_config("oauth_pending_chat_id") {
        if let Ok(cid) = s.parse::<i64>() {
            if cid != 0 {
                let tok = state.bot_token.clone();
                tokio::spawn(async move {
                    let _ = send_reply(
                        &tok,
                        cid,
                        "✅ Gmail connected! Trusty Izzie will now sync your sent mail.",
                    )
                    .await;
                });
                let _ = state.sqlite.set_config("oauth_pending_chat_id", "0");
            }
        }
    }

    info!("Google OAuth callback completed successfully");
    (
        StatusCode::OK,
        axum::response::Html(
            "<html><body><h2>Authenticated!</h2><p>Trusty Izzie is now connected to Gmail.</p><p>A confirmation was sent to your Telegram. You can close this tab.</p></body></html>"
                .to_string(),
        ),
    )
}

async fn webhook_handler(
    State(state): State<Arc<WebhookState>>,
    Json(update): Json<IncomingUpdate>,
) -> StatusCode {
    let msg = match update.message {
        Some(m) => m,
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

    // Persist chat_id for proactive daemon notifications (e.g., NeedsReauth).
    let _ = state
        .sqlite
        .set_config("telegram_primary_chat_id", &chat_id.to_string());

    // Handle document messages.
    if let Some(doc) = msg.document.clone() {
        let caption = msg.caption.clone().unwrap_or_default();
        let extractor = Arc::clone(&state.extractor);
        let store = Arc::clone(&state.store);
        let user_ctx = state.user_context.clone();
        let min_occ = state.min_occurrences;
        let gdrive_token = state.gdrive_token.clone();
        let token = state.bot_token.clone();

        tokio::spawn(async move {
            let source_ctx = format!(
                "telegram_file:{}",
                doc.file_name.as_deref().unwrap_or("unknown")
            );
            // Download the document bytes via the Telegram getFile API.
            let doc_text = download_and_extract_document_text(&token, &doc, chat_id).await;

            if let Some(text) = doc_text {
                let combined = if caption.is_empty() {
                    text
                } else {
                    format!("{}\n\n{}", caption, text)
                };
                if let Ok(result) = extractor
                    .extract_from_text(&combined, &source_ctx, &user_ctx)
                    .await
                {
                    if let Ok(stats) = persist_extraction_result(&result, &store, min_occ).await {
                        info!(
                            entities = stats.entities_written,
                            staged = stats.entities_staged,
                            rels = stats.relationships_written,
                            source = %source_ctx,
                            "ER extraction from document"
                        );
                        if let Some(token) = gdrive_token {
                            for entity in &result.entities {
                                spawn_drive_enrichment(
                                    entity.clone(),
                                    token.clone(),
                                    Arc::clone(&store),
                                );
                            }
                        }
                    }
                }
            }
        });
        return StatusCode::OK;
    }

    let text = match msg.text {
        Some(t) => t,
        None => return StatusCode::OK,
    };

    // Handle /auth command — generate PKCE link and send to user.
    if text.trim() == "/auth" || text.trim().starts_with("/auth ") {
        let (verifier, challenge) = generate_pkce_pair();
        let client_id = std::env::var("GOOGLE_CLIENT_ID").unwrap_or_default();
        let client_secret = std::env::var("GOOGLE_CLIENT_SECRET").unwrap_or_default();
        let ngrok =
            std::env::var("TRUSTY_NGROK_DOMAIN").unwrap_or_else(|_| "izzie.ngrok.dev".to_string());
        let redirect_uri = format!("https://{ngrok}/api/auth/google/callback");
        let auth_client = GoogleAuthClient::new(client_id, client_secret, redirect_uri);
        let auth_url = auth_client.authorization_url_pkce(&challenge);
        let _ = state.sqlite.set_config("oauth_pkce_verifier", &verifier);
        let _ = state
            .sqlite
            .set_config("oauth_pending_chat_id", &chat_id.to_string());
        let token = state.bot_token.clone();
        tokio::spawn(async move {
            let _ = send_reply(
                &token,
                chat_id,
                &format!(
                    "🔐 Authenticate trusty-izzie with Gmail:\n\n{auth_url}\n\nLink expires in ~10 minutes."
                ),
            )
            .await;
        });
        return StatusCode::OK;
    }

    // Process chat asynchronously — respond 200 immediately to Telegram.
    let engine = Arc::clone(&state.engine);
    let token = state.bot_token.clone();
    let extractor = Arc::clone(&state.extractor);
    let store = Arc::clone(&state.store);
    let user_ctx = state.user_context.clone();
    let min_occ = state.min_occurrences;
    let gdrive_token = state.gdrive_token.clone();
    let text_clone = text.clone();

    tokio::spawn(async move {
        let session_key = format!("tg_{chat_id}");
        let mut session = SessionManager::new_session(&session_key);

        // Check for Google Doc URL in the message text.
        let gdoc_text = if let Some(ref drive_token) = gdrive_token {
            extract_gdoc_text(&text_clone, drive_token).await
        } else {
            None
        };

        let full_text = match gdoc_text {
            Some(ref doc_text) => format!("{}\n\n{}", text_clone, doc_text),
            None => text_clone.clone(),
        };

        match engine.chat(&mut session, &text_clone).await {
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

        // Fire-and-forget ER extraction from the chat message.
        if let Ok(result) = extractor
            .extract_from_text(&full_text, "chat", &user_ctx)
            .await
        {
            if let Ok(stats) = persist_extraction_result(&result, &store, min_occ).await {
                info!(
                    entities = stats.entities_written,
                    staged = stats.entities_staged,
                    rels = stats.relationships_written,
                    "ER extraction from chat"
                );
                if let Some(token) = gdrive_token {
                    for entity in &result.entities {
                        spawn_drive_enrichment(entity.clone(), token.clone(), Arc::clone(&store));
                    }
                }
            }
        }
    });

    StatusCode::OK
}

/// Detect a Google Doc URL in text and export its content.
async fn extract_gdoc_text(text: &str, access_token: &str) -> Option<String> {
    // Match https://docs.google.com/document/d/{id}/...
    let re = regex::Regex::new(r"https://docs\.google\.com/document/d/([A-Za-z0-9_-]+)").ok()?;
    let caps = re.captures(text)?;
    let file_id = caps.get(1)?.as_str();

    let ch = gdrive::GDriveChannel::new(access_token.to_string());
    match ch.export_doc_text(file_id).await {
        Ok(text) => Some(text),
        Err(e) => {
            warn!(file_id = %file_id, error = %e, "failed to export Google Doc");
            None
        }
    }
}

/// Download a Telegram document and extract its text content.
///
/// Supports PDF (via lopdf), DOCX (via docx-rs), and plain text.
async fn download_and_extract_document_text(
    bot_token: &str,
    doc: &IncomingDocument,
    _chat_id: i64,
) -> Option<String> {
    // Step 1: Get file path from Telegram.
    let client = reqwest::Client::new();
    let file_info: serde_json::Value = client
        .get(format!("https://api.telegram.org/bot{}/getFile", bot_token))
        .query(&[("file_id", &doc.file_id)])
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    let file_path = file_info["result"]["file_path"].as_str()?;

    // Step 2: Download file bytes.
    let bytes = client
        .get(format!(
            "https://api.telegram.org/file/bot{}/{}",
            bot_token, file_path
        ))
        .send()
        .await
        .ok()?
        .bytes()
        .await
        .ok()?;

    let mime = doc
        .mime_type
        .as_deref()
        .unwrap_or("application/octet-stream");
    let name = doc.file_name.as_deref().unwrap_or("");

    // Step 3: Extract text based on MIME type.
    if mime == "application/pdf" || name.ends_with(".pdf") {
        extract_pdf_text(&bytes)
    } else if mime == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        || name.ends_with(".docx")
    {
        extract_docx_text(&bytes)
    } else {
        // Try as plain UTF-8 text.
        String::from_utf8(bytes.to_vec()).ok()
    }
}

/// Extract plain text from PDF bytes using lopdf.
fn extract_pdf_text(bytes: &[u8]) -> Option<String> {
    use lopdf::Document;

    let doc = Document::load_mem(bytes).ok()?;
    let page_numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
    let text = doc.extract_text(&page_numbers).ok()?;
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Extract plain text from DOCX bytes using docx-rs.
fn extract_docx_text(bytes: &[u8]) -> Option<String> {
    let docx = docx_rs::read_docx(bytes).ok()?;
    let mut text = String::new();
    for child in &docx.document.children {
        if let docx_rs::DocumentChild::Paragraph(para) = child {
            for run_child in &para.children {
                if let docx_rs::ParagraphChild::Run(run) = run_child {
                    for run_content in &run.children {
                        if let docx_rs::RunChild::Text(t) = run_content {
                            text.push_str(&t.text);
                        }
                    }
                }
            }
            text.push('\n');
        }
    }
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_webhook(
    bot_token: String,
    webhook_url: String,
    port: u16,
    engine: Arc<ChatEngine>,
    allowed_users: Vec<i64>,
    sqlite: Arc<SqliteStore>,
    extractor: Arc<EntityExtractor>,
    store: Arc<Store>,
    user_context: UserContext,
    min_occurrences: u32,
    gdrive_token: Option<String>,
) -> Result<()> {
    // Register webhook with Telegram.
    api_set_webhook(&bot_token, &webhook_url).await?;

    let state = Arc::new(WebhookState {
        engine,
        allowed_users,
        bot_token,
        sqlite,
        extractor,
        store,
        user_context,
        min_occurrences,
        gdrive_token,
    });

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/webhook/telegram", post(webhook_handler))
        .route("/api/auth/google/callback", get(oauth_callback_handler))
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
            // Prefer the explicitly paired token in SQLite over ambient env vars,
            // so that AI Commander's TELEGRAM_BOT_TOKEN doesn't bleed in.
            let token = sqlite
                .get_config("telegram_bot_token")
                .ok()
                .flatten()
                .or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok())
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

            const INSTANCE_ID: &str = "42a923e9bd673e38";
            let store = Arc::new(Store::open(&data_dir, INSTANCE_ID).await?);
            let assembler = ContextAssembler::new(5, 10).with_lance(Arc::clone(&store.lance));

            let api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();

            let engine = Arc::new(ChatEngine::new_with_context(
                config.openrouter.base_url.clone(),
                api_key.clone(),
                config.openrouter.chat_model.clone(),
                config.chat.max_tool_iterations,
                assembler,
            ));

            // Build the entity extractor.
            let extractor = Arc::new(EntityExtractor::new(ExtractorConfig {
                base_url: config.openrouter.base_url.clone(),
                api_key: api_key.clone(),
                model: config.openrouter.extraction_model.clone(),
                max_tokens: 2048,
                confidence_threshold: 0.85,
                max_relationships: 3,
            }));

            // Build user context from environment / config.
            let primary_email = std::env::var("TRUSTY_PRIMARY_EMAIL")
                .unwrap_or_else(|_| "bobmatnyc@gmail.com".to_string());
            let user_context = UserContext {
                user_id: INSTANCE_ID.to_string(),
                email: primary_email.clone(),
                display_name: primary_email.clone(),
            };

            let min_occurrences = 2u32;
            let gdrive_token = sqlite.get_config("google_access_token").ok().flatten();

            // Spawn the background email sync loop.
            {
                let store_sync = Arc::clone(&store);
                let extractor_sync = Arc::clone(&extractor);
                let user_ctx_sync = user_context.clone();
                let _sync_handle = tokio::task::spawn(async move {
                    email_sync::run_email_sync_loop(
                        store_sync,
                        extractor_sync,
                        user_ctx_sync,
                        3600,
                        min_occurrences,
                    )
                    .await;
                });
            }

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
                run_webhook(
                    token,
                    url,
                    port,
                    engine,
                    allowed,
                    Arc::clone(&store.sqlite),
                    extractor,
                    store,
                    user_context,
                    min_occurrences,
                    gdrive_token,
                )
                .await?;
            }
        }

        Command::Webhook { action } => {
            // Prefer the explicitly paired token in SQLite over ambient env vars.
            let token = sqlite
                .get_config("telegram_bot_token")
                .ok()
                .flatten()
                .or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok())
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
