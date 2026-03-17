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
mod er_persist;
mod gdrive;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use axum::extract::{Json, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use clap::{Parser, Subcommand};
use serde::Deserialize;
use teloxide::prelude::*;
use teloxide::types::ChatAction;
use tracing::{error, info, warn};

use trusty_chat::{context::ContextAssembler, engine::ChatEngine, session::SessionManager};
use trusty_email::auth::{generate_pkce_pair, GoogleAuthClient};
use trusty_embeddings::{Embedder, EmbeddingModel};
use trusty_extractor::{EntityExtractor, ExtractorConfig, UserContext};
use trusty_memory::{MemoryRecaller, MemoryStore};
use trusty_metro_north::MetroNorthSkill;
use trusty_store::sqlite::SqliteStore;
use trusty_store::Store;
use trusty_weather::WeatherSkill;

use er_persist::persist_extraction_result;
use gdrive::spawn_drive_enrichment;

// ---------------------------------------------------------------------------
// Instance ID
// ---------------------------------------------------------------------------

/// Load or generate a persistent instance ID.
///
/// Resolution order:
/// 1. `TRUSTY_INSTANCE_ID` env var
/// 2. `~/.local/share/trusty-izzie/instance.json` → `"instance_id"` field
/// 3. Generate a random 16-hex-char string and write it to the file
fn load_instance_id() -> String {
    if let Ok(id) = std::env::var("TRUSTY_INSTANCE_ID") {
        if !id.is_empty() {
            return id;
        }
    }
    let path = {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home).join(".local/share/trusty-izzie/instance.json")
    };
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if let Some(id) = val.get("instance_id").and_then(|v| v.as_str()) {
                if !id.is_empty() {
                    return id.to_string();
                }
            }
        }
    }
    let id = format!("{:016x}", uuid::Uuid::new_v4().as_u128() as u64);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, serde_json::json!({"instance_id": id}).to_string());
    id
}

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
        /// Start HTTP server only — no Telegram connection (for smoke testing / CI).
        #[arg(long)]
        http_only: bool,
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
async fn api_set_webhook(
    client: &reqwest::Client,
    token: &str,
    url: &str,
    secret_token: Option<&str>,
) -> Result<()> {
    let endpoint = format!("https://api.telegram.org/bot{token}/setWebhook");
    let mut body = serde_json::json!({ "url": url });
    if let Some(s) = secret_token {
        body["secret_token"] = serde_json::json!(s);
    }
    let resp: serde_json::Value = client
        .post(&endpoint)
        .json(&body)
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
async fn api_delete_webhook(client: &reqwest::Client, token: &str) -> Result<()> {
    let endpoint = format!("https://api.telegram.org/bot{token}/deleteWebhook");
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

/// Send a chat action (e.g. "typing"). Fire-and-forget — never fails the caller.
/// Reverse-geocode lat/lon to a human-readable place name via Nominatim.
/// Returns e.g. "Berlin, Germany" or falls back to raw coordinates.
async fn reverse_geocode(client: &reqwest::Client, lat: f64, lon: f64) -> String {
    let url = format!(
        "https://nominatim.openstreetmap.org/reverse?format=json&lat={:.2}&lon={:.2}&zoom=10",
        lat, lon
    );
    let resp = client
        .get(&url)
        .header("User-Agent", "trusty-izzie/0.1 (personal assistant)")
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;
    if let Ok(r) = resp {
        if let Ok(json) = r.json::<serde_json::Value>().await {
            let city = json["address"]["city"]
                .as_str()
                .or_else(|| json["address"]["town"].as_str())
                .or_else(|| json["address"]["village"].as_str())
                .or_else(|| json["address"]["county"].as_str())
                .unwrap_or("");
            let country = json["address"]["country"].as_str().unwrap_or("");
            if !city.is_empty() && !country.is_empty() {
                return format!("{}, {}", city, country);
            } else if !country.is_empty() {
                return country.to_string();
            } else if let Some(name) = json["display_name"].as_str() {
                // Trim to first two comma-separated parts for brevity
                let parts: Vec<&str> = name.splitn(3, ',').collect();
                return parts[..parts.len().min(2)].join(",").trim().to_string();
            }
        }
    }
    format!("{:.4}°, {:.4}°", lat, lon)
}

async fn send_chat_action(client: &reqwest::Client, token: &str, chat_id: i64, action: &str) {
    let endpoint = format!("https://api.telegram.org/bot{token}/sendChatAction");
    let _ = client
        .post(&endpoint)
        .json(&serde_json::json!({"chat_id": chat_id, "action": action}))
        .send()
        .await;
}

/// Send a message. Returns the Telegram message_id for later editing.
/// Uses a two-attempt approach: try with parse_mode first, then plain text on 400.
async fn send_message(
    client: &reqwest::Client,
    token: &str,
    chat_id: i64,
    text: &str,
    reply_to_message_id: Option<i64>,
    parse_mode: &str,
) -> Result<i64> {
    let endpoint = format!("https://api.telegram.org/bot{token}/sendMessage");
    let mut body = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
        "disable_web_page_preview": true,
    });
    if !parse_mode.is_empty() {
        body["parse_mode"] = serde_json::Value::String(parse_mode.to_string());
    }
    if let Some(rid) = reply_to_message_id {
        body["reply_to_message_id"] = serde_json::Value::Number(rid.into());
    }
    let resp: serde_json::Value = client
        .post(&endpoint)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;
    if !resp["ok"].as_bool().unwrap_or(false) {
        // If parse_mode failed with a 400, retry as plain text.
        if !parse_mode.is_empty() && resp["error_code"].as_i64() == Some(400) {
            let mut plain_body = serde_json::json!({
                "chat_id": chat_id,
                "text": text,
                "disable_web_page_preview": true,
            });
            if let Some(rid) = reply_to_message_id {
                plain_body["reply_to_message_id"] = serde_json::Value::Number(rid.into());
            }
            let plain_resp: serde_json::Value = client
                .post(&endpoint)
                .json(&plain_body)
                .send()
                .await?
                .json()
                .await?;
            if !plain_resp["ok"].as_bool().unwrap_or(false) {
                let desc = plain_resp["description"].as_str().unwrap_or("unknown");
                return Err(anyhow::anyhow!("sendMessage failed: {desc}"));
            }
            return Ok(plain_resp["result"]["message_id"].as_i64().unwrap_or(0));
        }
        let desc = resp["description"].as_str().unwrap_or("unknown");
        return Err(anyhow::anyhow!("sendMessage failed: {desc}"));
    }
    Ok(resp["result"]["message_id"].as_i64().unwrap_or(0))
}

/// Edit an existing message (for progress updates -> final reply).
async fn edit_message_text(
    client: &reqwest::Client,
    token: &str,
    chat_id: i64,
    message_id: i64,
    text: &str,
    parse_mode: &str,
) -> Result<()> {
    let endpoint = format!("https://api.telegram.org/bot{token}/editMessageText");
    let mut body = serde_json::json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "text": text,
        "disable_web_page_preview": true,
    });
    if !parse_mode.is_empty() {
        body["parse_mode"] = serde_json::Value::String(parse_mode.to_string());
    }
    let resp: serde_json::Value = client
        .post(&endpoint)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;
    if !resp["ok"].as_bool().unwrap_or(false) {
        // If parse fails, retry plain.
        if !parse_mode.is_empty() && resp["error_code"].as_i64() == Some(400) {
            let plain_body = serde_json::json!({
                "chat_id": chat_id,
                "message_id": message_id,
                "text": text,
                "disable_web_page_preview": true,
            });
            let _ = client.post(&endpoint).json(&plain_body).send().await;
            return Ok(());
        }
        let desc = resp["description"].as_str().unwrap_or("unknown");
        warn!("editMessageText failed: {desc}");
    }
    Ok(())
}

/// Delete a message (e.g. remove progress placeholder).
#[allow(dead_code)]
async fn delete_message(client: &reqwest::Client, token: &str, chat_id: i64, message_id: i64) {
    let endpoint = format!("https://api.telegram.org/bot{token}/deleteMessage");
    let _ = client
        .post(&endpoint)
        .json(&serde_json::json!({"chat_id": chat_id, "message_id": message_id}))
        .send()
        .await;
}

/// Split text into chunks <= max_len bytes, breaking on paragraph boundaries where possible.
fn chunk_text(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }
        let safe_len = remaining.floor_char_boundary(max_len);
        let window = &remaining[..safe_len];
        let split_at = window
            .rfind("\n\n")
            .or_else(|| window.rfind('\n'))
            .or_else(|| window.rfind(". "))
            .unwrap_or(max_len);
        let (chunk, rest) = remaining.split_at(split_at);
        chunks.push(chunk.trim_end().to_string());
        remaining = rest.trim_start();
    }
    chunks
}

/// Send a (potentially long) reply, splitting into multiple messages if needed.
/// Uses HTML parse_mode; falls back to plain text if HTML parse fails.
/// Returns the message_id of the LAST sent message.
async fn send_reply_smart(
    client: &reqwest::Client,
    token: &str,
    chat_id: i64,
    text: &str,
    reply_to_message_id: Option<i64>,
) -> Result<i64> {
    const MAX: usize = 4000;
    let chunks = chunk_text(text, MAX);
    let mut last_id = 0i64;
    for (i, chunk) in chunks.iter().enumerate() {
        let rid = if i == 0 { reply_to_message_id } else { None };
        last_id = match send_message(client, token, chat_id, chunk, rid, "HTML").await {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(error = %e, "send_reply_smart: failed to send chunk, continuing");
                0
            }
        };
    }
    Ok(last_id)
}

/// Backward-compat wrapper used for simple notifications (auth, errors, etc.)
async fn send_reply(client: &reqwest::Client, token: &str, chat_id: i64, text: &str) -> Result<()> {
    send_message(client, token, chat_id, text, None, "")
        .await
        .map(|_| ())
}

/// Send a "progress" placeholder message; returns its message_id for later editing.
#[allow(dead_code)]
async fn send_progress_message(
    client: &reqwest::Client,
    token: &str,
    chat_id: i64,
    reply_to: i64,
) -> i64 {
    send_message(client, token, chat_id, "…", Some(reply_to), "")
        .await
        .unwrap_or(0)
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
struct ReplyMessage {
    text: Option<String>,
    caption: Option<String>,
}

#[derive(Deserialize)]
struct IncomingMessage {
    message_id: i64,
    chat: IncomingChat,
    from: Option<IncomingUser>,
    text: Option<String>,
    document: Option<IncomingDocument>,
    caption: Option<String>,
    location: Option<IncomingLocation>,
    reply_to_message: Option<ReplyMessage>,
}

#[derive(Deserialize)]
struct IncomingLocation {
    latitude: f64,
    longitude: f64,
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
    username: Option<String>,
}

/// Query parameters for the Google OAuth callback route.
#[derive(serde::Deserialize)]
struct OAuthCallbackQuery {
    code: Option<String>,
    error: Option<String>,
    state: Option<String>,
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
    memory_store: Arc<MemoryStore>,
    session_manager: Arc<SessionManager>,
    http: reqwest::Client,
    google_client_id: String,
    google_client_secret: String,
}

async fn health_handler() -> StatusCode {
    StatusCode::OK
}

// ---------------------------------------------------------------------------
// HTTP chat endpoint — allows trusty-cli to delegate to this process and avoid
// opening a competing KuzuDB write connection.
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct ChatRequest {
    message: String,
    #[serde(default)]
    session_id: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct ChatResponse {
    reply: String,
    session_id: String,
}

async fn chat_handler(
    State(state): State<Arc<WebhookState>>,
    Json(req): Json<ChatRequest>,
) -> impl axum::response::IntoResponse {
    let session_id = req
        .session_id
        .as_deref()
        .and_then(|s| s.parse::<uuid::Uuid>().ok())
        .unwrap_or_else(uuid::Uuid::new_v4);

    // Load existing session or create a fresh one for the CLI user (last 50 messages).
    let mut session = match state.session_manager.load_or_create(session_id, 50).await {
        Ok(s) => s,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };

    match state.engine.chat(&mut session, &req.message).await {
        Ok(response) => {
            // Persist session (best-effort, async).
            let sm = Arc::clone(&state.session_manager);
            let session_to_save = session.clone();
            tokio::spawn(async move {
                if let Err(e) = sm.save(&session_to_save).await {
                    warn!("cli chat: failed to save session: {e}");
                }
            });

            // Persist memories (best-effort, async).
            if !response.memories_to_save.is_empty() {
                let mem_store = Arc::clone(&state.memory_store);
                let memories = response.memories_to_save.clone();
                tokio::spawn(async move {
                    for mem in memories {
                        if let Err(e) = mem_store
                            .save(
                                "cli",
                                &mem.content,
                                mem.category,
                                mem.related_entities,
                                mem.importance,
                                None,
                            )
                            .await
                        {
                            warn!("cli chat: failed to save memory: {e}");
                        }
                    }
                });
            }

            (
                StatusCode::OK,
                Json(ChatResponse {
                    reply: response.reply,
                    session_id: session_id.to_string(),
                }),
            )
                .into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
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

    // Validate OAuth CSRF state parameter.
    let stored_state = state
        .sqlite
        .get_config("oauth_pending_state")
        .ok()
        .flatten()
        .unwrap_or_default();
    if !stored_state.is_empty() {
        let provided_state = params.state.as_deref().unwrap_or("");
        if provided_state != stored_state {
            warn!("oauth_callback: state mismatch — possible CSRF attack");
            return (
                StatusCode::BAD_REQUEST,
                axum::response::Html(
                    "<html><body><h2>Invalid state parameter.</h2></body></html>".to_string(),
                ),
            );
        }
    }
    let _ = state.sqlite.set_config("oauth_pending_state", "");

    let client_id = state.google_client_id.clone();
    let client_secret = state.google_client_secret.clone();
    let ngrok_domain =
        std::env::var("TRUSTY_NGROK_DOMAIN").unwrap_or_else(|_| "izzie.ngrok.dev".to_string());
    let redirect_uri = format!("https://{}/api/auth/google/callback", ngrok_domain);

    let resp = state
        .http
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("code", code.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
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

    // Resolve the authenticated email via Google userinfo.
    let auth_email = match state
        .http
        .get("https://www.googleapis.com/oauth2/v3/userinfo")
        .bearer_auth(&access_token)
        .send()
        .await
    {
        Ok(r) => {
            let info: serde_json::Value = r.json().await.unwrap_or_default();
            info["email"]
                .as_str()
                .unwrap_or("unknown@example.com")
                .to_string()
        }
        Err(e) => {
            warn!("Could not fetch userinfo: {e}");
            "unknown@example.com".to_string()
        }
    };
    // Fall back to the identity hint stored during /auth if userinfo didn't return an email.
    let auth_email = if auth_email == "unknown@example.com" {
        state
            .sqlite
            .get_config("oauth_pending_identity_email")
            .ok()
            .flatten()
            .filter(|e| !e.is_empty() && e != "unknown@example.com")
            .unwrap_or(auth_email)
    } else {
        auth_email
    };
    let _ = state.sqlite.set_config("oauth_pending_identity_email", "");

    // Store in oauth_tokens table (proper per-account path).
    let expires_at = body["expires_in"]
        .as_i64()
        .map(|secs| chrono::Utc::now().timestamp() + secs);
    let granted_scopes = body["scope"]
        .as_str()
        .unwrap_or("https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/gmail.send")
        .to_string();
    if let Err(e) = state.sqlite.upsert_oauth_token(
        &auth_email,
        &access_token,
        if refresh_token.is_empty() {
            None
        } else {
            Some(refresh_token.as_str())
        },
        expires_at,
        Some(granted_scopes.as_str()),
    ) {
        error!("Failed to store oauth_token for {auth_email}: {e}");
    }

    // Register account (primary or secondary).
    let primary_email = std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_default();
    let account_type = if auth_email == primary_email {
        "primary"
    } else {
        "secondary"
    };
    if let Err(e) = state
        .sqlite
        .add_account(&auth_email, Some(&auth_email), account_type)
    {
        error!("Failed to register account {auth_email}: {e}");
    }

    // Backward compat: also write kv_config for the primary account (sync loop reads this).
    if auth_email == primary_email {
        if let Err(e) = state
            .sqlite
            .set_config("google_access_token", &access_token)
        {
            error!("Failed to store access token in kv_config: {e}");
        }
        if !refresh_token.is_empty() {
            if let Err(e) = state
                .sqlite
                .set_config("google_refresh_token", &refresh_token)
            {
                error!("Failed to store refresh token in kv_config: {e}");
            }
        }
    }

    // Clear the one-time verifier.
    let _ = state.sqlite.set_config("oauth_pkce_verifier", "");

    // Notify the user in Telegram if we have a pending chat_id.
    if let Ok(Some(s)) = state.sqlite.get_config("oauth_pending_chat_id") {
        if let Ok(cid) = s.parse::<i64>() {
            if cid != 0 {
                // Store email so the next message (work/personal reply) can look it up.
                let _ = state
                    .sqlite
                    .set_config("oauth_pending_identity_email", &auth_email);
                let tok = state.bot_token.clone();
                let http_notify = state.http.clone();
                let auth_email_notify = auth_email.clone();
                tokio::spawn(async move {
                    let _ = send_message(
                        &http_notify,
                        &tok,
                        cid,
                        &format!(
                            "✅ Connected <b>{auth_email_notify}</b>!\n\nIs this your <b>work</b> or <b>personal</b> account?\nReply with <code>work</code> or <code>personal</code>."
                        ),
                        None,
                        "HTML",
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
            format!("<html><body><h2>Authenticated!</h2><p>Trusty Izzie is now connected to Gmail as <strong>{auth_email}</strong>.</p><p>A confirmation was sent to your Telegram. You can close this tab.</p></body></html>"),
        ),
    )
}

async fn webhook_handler(
    State(state): State<Arc<WebhookState>>,
    headers: axum::http::HeaderMap,
    Json(update): Json<IncomingUpdate>,
) -> StatusCode {
    // Validate the Telegram webhook secret token.
    let expected = state
        .sqlite
        .get_config("webhook_secret_token")
        .unwrap_or_default()
        .unwrap_or_default();
    let provided = headers
        .get("X-Telegram-Bot-Api-Secret-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !expected.is_empty() && provided != expected {
        warn!("webhook: rejected request with invalid secret token");
        return StatusCode::FORBIDDEN;
    }
    let msg = match update.message {
        Some(m) => m,
        None => return StatusCode::OK,
    };

    let chat_id = msg.chat.id;
    let message_id = msg.message_id;
    let sender_user_id = msg.from.as_ref().map(|u| u.id);
    let sender_username = msg
        .from
        .as_ref()
        .and_then(|u| u.username.as_deref())
        .map(str::to_string);

    // Authorisation check
    if !state.allowed_users.is_empty() {
        let uid = sender_user_id.unwrap_or(0);
        if !state.allowed_users.contains(&uid) {
            let token = state.bot_token.clone();
            let http = state.http.clone();
            tokio::spawn(async move {
                let _ = send_reply(&http, &token, chat_id, "Not authorized.").await;
            });
            return StatusCode::OK;
        }
    }

    // Log inbound message.
    let inbound_text = msg
        .text
        .as_deref()
        .or(msg.caption.as_deref())
        .unwrap_or("[document]");
    if let Err(e) = state.sqlite.log_telegram_interaction(
        "inbound",
        chat_id,
        sender_user_id,
        sender_username.as_deref(),
        inbound_text,
        None,
    ) {
        warn!("Failed to log inbound telegram message: {e}");
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
        let http_doc = state.http.clone();

        tokio::spawn(async move {
            let source_ctx = format!(
                "telegram_file:{}",
                doc.file_name.as_deref().unwrap_or("unknown")
            );
            // Download the document bytes via the Telegram getFile API.
            let doc_text =
                download_and_extract_document_text(&http_doc, &token, &doc, chat_id).await;

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

    // Handle Telegram location share (GPS coordinates).
    if let Some(loc) = &msg.location {
        let place = reverse_geocode(&state.http, loc.latitude, loc.longitude).await;
        let memory_content = format!("User's current location: {}", place);
        let sqlite_loc = state.sqlite.clone();
        let place_clone = place.clone();
        let mem_store_loc = Arc::clone(&state.memory_store);
        let user_id_loc = state.user_context.user_id.clone();
        let token_loc = state.bot_token.clone();
        let http_loc = state.http.clone();
        tokio::spawn(async move {
            // Persist location as a short-lived memory and as a kv_config entry.
            let _ = mem_store_loc
                .save(
                    &user_id_loc,
                    &memory_content,
                    trusty_models::memory::MemoryCategory::Location,
                    vec![],
                    0.9,
                    None,
                )
                .await;
            let place_for_ack = place_clone.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _ = sqlite_loc.set_config("user_current_location", &place_clone);
            })
            .await;
            // Acknowledge the location share.
            let ack = format!("📍 Got it — I've noted you're in <b>{}</b>.", place_for_ack);
            let _ = send_message(
                &http_loc,
                &token_loc,
                chat_id,
                &ack,
                Some(message_id),
                "HTML",
            )
            .await;
        });
        return StatusCode::OK;
    }

    let reply_context = msg.reply_to_message;
    let text = match msg.text {
        Some(t) => t,
        None => return StatusCode::OK,
    };

    // 1. Show typing indicator immediately (fire-and-forget).
    {
        let tok_clone = state.bot_token.clone();
        let http_typing = state.http.clone();
        let chat_id_copy = chat_id;
        tokio::spawn(async move {
            send_chat_action(&http_typing, &tok_clone, chat_id_copy, "typing").await;
        });
    }

    // Handle /auth command — generate PKCE link and send to user.
    if text.trim() == "/auth" || text.trim().starts_with("/auth ") {
        // Extract optional email hint: /auth user@example.com
        let login_hint_email: Option<&str> = text
            .trim()
            .strip_prefix("/auth ")
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let (verifier, challenge) = generate_pkce_pair();
        let state_value: String = {
            use rand::Rng;
            rand::thread_rng()
                .sample_iter(&rand::distributions::Alphanumeric)
                .take(32)
                .map(char::from)
                .collect()
        };
        let client_id = state.google_client_id.clone();
        let client_secret = state.google_client_secret.clone();
        let ngrok =
            std::env::var("TRUSTY_NGROK_DOMAIN").unwrap_or_else(|_| "izzie.ngrok.dev".to_string());
        let redirect_uri = format!("https://{ngrok}/api/auth/google/callback");
        let auth_client = GoogleAuthClient::new(client_id, client_secret, redirect_uri);
        let auth_url = format!(
            "{}&state={}",
            auth_client.authorization_url_pkce(&challenge),
            state_value
        );
        let _ = state.sqlite.set_config("oauth_pkce_verifier", &verifier);
        let _ = state.sqlite.set_config("oauth_pending_state", &state_value);
        let _ = state
            .sqlite
            .set_config("oauth_pending_chat_id", &chat_id.to_string());
        // Store identity hint so callback can identify account if userinfo fails.
        if let Some(hint) = login_hint_email {
            let _ = state
                .sqlite
                .set_config("oauth_pending_identity_email", hint);
        }
        let token = state.bot_token.clone();
        let http_auth = state.http.clone();
        tokio::spawn(async move {
            let endpoint = format!("https://api.telegram.org/bot{token}/sendMessage");
            let body = serde_json::json!({
                "chat_id": chat_id,
                "text": "Tap the button below to authorize your Google account:",
                "reply_markup": {
                    "inline_keyboard": [[{
                        "text": "Authorize Google Account",
                        "url": auth_url
                    }]]
                }
            });
            let _ = http_auth.post(&endpoint).json(&body).send().await;
        });
        return StatusCode::OK;
    }

    // Intercept work/personal identity reply after OAuth.
    if let Ok(Some(pending_email)) = state.sqlite.get_config("oauth_pending_identity_email") {
        if !pending_email.is_empty() {
            let reply_text = text.trim().to_lowercase();
            if reply_text == "work" || reply_text == "personal" {
                if let Err(e) = state
                    .sqlite
                    .update_account_identity(&pending_email, &reply_text)
                {
                    warn!("Failed to update account identity: {e}");
                }
                let _ = state.sqlite.set_config("oauth_pending_identity_email", "");
                let label = if reply_text == "work" {
                    "work"
                } else {
                    "personal"
                };
                let purpose = if reply_text == "work" {
                    "work calendar, email, and tasks"
                } else {
                    "personal calendar, email, and tasks"
                };
                let ack = format!(
                    "Got it — <b>{pending_email}</b> is your {label} account. I'll use it for {purpose}."
                );
                let tok = state.bot_token.clone();
                let http_ident = state.http.clone();
                tokio::spawn(async move {
                    let _ =
                        send_message(&http_ident, &tok, chat_id, &ack, Some(message_id), "HTML")
                            .await;
                });
                return StatusCode::OK;
            }
        }
    }

    // Process chat asynchronously — respond 200 immediately to Telegram.
    let engine = Arc::clone(&state.engine);
    let token = state.bot_token.clone();
    let http_chat = state.http.clone();
    let extractor = Arc::clone(&state.extractor);
    let store = Arc::clone(&state.store);
    let sqlite_log = Arc::clone(&state.sqlite);
    let user_ctx = state.user_context.clone();
    let min_occ = state.min_occurrences;
    let gdrive_token = state.gdrive_token.clone();
    let text_clone = if let Some(ref reply) = reply_context {
        let reply_text = reply
            .text
            .as_deref()
            .or(reply.caption.as_deref())
            .unwrap_or("");
        if !reply_text.is_empty() {
            format!("[Replying to: \"{}\"]\n{}", reply_text, text)
        } else {
            text.clone()
        }
    } else {
        text.clone()
    };
    let memory_store = Arc::clone(&state.memory_store);
    let memory_user_id = state.user_context.user_id.clone();
    let session_manager = Arc::clone(&state.session_manager);

    tokio::spawn(async move {
        // 2. No placeholder message — typing indicator in the header is sufficient.
        let progress_id: i64 = 0;

        // Handle /start and /help commands.
        if text_clone.trim() == "/start" || text_clone.trim() == "/help" {
            let help_text = concat!(
                "👋 <b>Trusty Izzie</b> — your personal AI assistant\n\n",
                "<b>Commands:</b>\n",
                "/auth — Connect or reconnect a Google account\n",
                "/help — Show this message\n\n",
                "<b>What I can do:</b>\n",
                "• Answer questions about your contacts and relationships\n",
                "• Schedule reminders and events\n",
                "• Manage email account syncing\n",
                "• Run background research agents\n",
                "• Check service status\n\n",
                "Just chat naturally — no commands needed for most things."
            );
            if progress_id > 0 {
                let _ =
                    edit_message_text(&http_chat, &token, chat_id, progress_id, help_text, "HTML")
                        .await;
            } else {
                let _ = send_message(
                    &http_chat,
                    &token,
                    chat_id,
                    help_text,
                    Some(message_id),
                    "HTML",
                )
                .await;
            }
            return;
        }

        // Derive a stable session UUID from chat_id so history persists across reboots.
        // Negative chat_ids (groups) wrap around safely in u128.
        let session_uuid = uuid::Uuid::from_u128(chat_id as u128);
        let session_mgr = Arc::clone(&session_manager);
        let mut session = session_mgr
            .load_or_create(session_uuid, 40)
            .await
            .unwrap_or_else(|_| SessionManager::new_session(&format!("tg_{chat_id}")));

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

        // 6. Spawn typing heartbeat — refreshes "typing" every 4s until cancelled.
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        {
            let tok_hb = token.clone();
            let http_hb = http_chat.clone();
            let chat_id_copy = chat_id;
            tokio::spawn(async move {
                let mut cancel_rx = cancel_rx;
                loop {
                    tokio::select! {
                        _ = &mut cancel_rx => break,
                        _ = tokio::time::sleep(tokio::time::Duration::from_secs(4)) => {
                            send_chat_action(&http_hb, &tok_hb, chat_id_copy, "typing").await;
                        }
                    }
                }
            });
        }

        // 3. Process LLM call.
        match engine.chat(&mut session, &text_clone).await {
            Ok(response) => {
                // Stop heartbeat.
                let _ = cancel_tx.send(());

                info!(
                    memories = response.memories_to_save.len(),
                    "Chat turn complete"
                );

                // Use a fallback if the LLM returns an empty reply.
                let reply_owned;
                let reply_text: &str = if response.reply.trim().is_empty() {
                    reply_owned = "👍".to_string();
                    &reply_owned
                } else {
                    &response.reply
                };
                // 4. Send reply (no placeholder message — typing indicator in header handles UX).
                let _ = send_reply_smart(&http_chat, &token, chat_id, reply_text, Some(message_id))
                    .await;

                if let Err(e) = sqlite_log
                    .log_telegram_interaction("outbound", chat_id, None, None, reply_text, None)
                {
                    warn!("Failed to log outbound telegram message: {e}");
                }

                // 5. Persist conversation session so history survives reboots.
                let sm_save = Arc::clone(&session_mgr);
                let session_to_save = session.clone();
                tokio::spawn(async move {
                    if let Err(e) = sm_save.save(&session_to_save).await {
                        warn!("Failed to save chat session: {e}");
                    }
                });

                if !response.memories_to_save.is_empty() {
                    let mem_store = Arc::clone(&memory_store);
                    let user_id = memory_user_id.clone();
                    let memories = response.memories_to_save.clone();
                    tokio::spawn(async move {
                        for mem in memories {
                            if let Err(e) = mem_store
                                .save(
                                    &user_id,
                                    &mem.content,
                                    mem.category,
                                    mem.related_entities,
                                    mem.importance,
                                    None,
                                )
                                .await
                            {
                                warn!("Failed to save memory: {e}");
                            }
                        }
                    });
                }
            }
            Err(e) => {
                // Stop heartbeat.
                let _ = cancel_tx.send(());
                let e_str = e.to_string();
                error!("Chat error: {e_str}");
                let err_text = if e_str.contains("402") || e_str.contains("Insufficient credits") {
                    "⚠️ OpenRouter out of credits. Add more at openrouter.ai/settings/credits"
                        .to_string()
                } else if e_str.contains("429")
                    || e_str.contains("rate limit")
                    || e_str.contains("Rate limit")
                {
                    "⚠️ OpenRouter rate limit hit. Try again in a moment.".to_string()
                } else if e_str.contains("500")
                    || e_str.contains("502")
                    || e_str.contains("503")
                    || e_str.contains("Internal Server Error")
                {
                    "⚠️ OpenRouter service error (500). Try again in a moment.".to_string()
                } else {
                    format!("⚠️ Error: {}", &e_str[..e_str.len().min(120)])
                };
                if progress_id > 0 {
                    let _ =
                        edit_message_text(&http_chat, &token, chat_id, progress_id, &err_text, "")
                            .await;
                } else {
                    let _ = send_reply(&http_chat, &token, chat_id, &err_text).await;
                }
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

        // Detect open-loop signals in the user's message and schedule follow-up.
        {
            let lower = text_clone.to_lowercase();
            let is_open_loop = lower.contains("will do")
                || lower.contains("i'll")
                || lower.contains("remind me")
                || lower.contains("follow up")
                || lower.contains("don't let me forget")
                || lower.contains("todo")
                || lower.contains("need to")
                || lower.contains("should do");

            if is_open_loop {
                let followup_hours = sqlite_log
                    .get_pref("open_loop_followup_hours")
                    .unwrap_or(None)
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(24);
                let followup_enabled = sqlite_log
                    .get_pref("open_loop_followup_enabled")
                    .unwrap_or(None)
                    .unwrap_or_else(|| "true".to_string())
                    == "true";

                if followup_enabled {
                    let follow_up_at = chrono::Utc::now().timestamp() + followup_hours * 3600;
                    let desc = text_clone.chars().take(200).collect::<String>();
                    let loop_id = uuid::Uuid::new_v4().to_string();
                    let sqlite_clone = sqlite_log.clone();
                    let lid = loop_id.clone();
                    let desc_clone = desc.clone();
                    tokio::task::spawn_blocking(move || {
                        let _ =
                            sqlite_clone.create_open_loop(&lid, &desc_clone, None, follow_up_at);
                        let _ = sqlite_clone.enqueue_event(
                            &trusty_models::EventType::FollowUp,
                            &trusty_models::EventPayload::FollowUp {
                                open_loop_id: lid.clone(),
                                description: desc_clone,
                            },
                            follow_up_at,
                            3,
                            1,
                            "system",
                            None,
                        );
                    })
                    .await
                    .ok();
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
    client: &reqwest::Client,
    bot_token: &str,
    doc: &IncomingDocument,
    _chat_id: i64,
) -> Option<String> {
    // Step 1: Get file path from Telegram.
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

async fn run_http_only(
    port: u16,
    engine: Arc<ChatEngine>,
    sqlite: Arc<SqliteStore>,
    store: Arc<Store>,
    memory_store: Arc<MemoryStore>,
) -> Result<()> {
    let http = reqwest::Client::new();
    let google_client_id = std::env::var("GOOGLE_CLIENT_ID").unwrap_or_default();
    let google_client_secret =
        trusty_core::secrets::get("GOOGLE_CLIENT_SECRET").unwrap_or_default();
    let session_manager = Arc::new(SessionManager::new(sqlite.clone()));
    let extractor = Arc::new(EntityExtractor::new(ExtractorConfig {
        base_url: String::new(),
        api_key: String::new(),
        model: String::new(),
        max_tokens: 0,
        confidence_threshold: 0.85,
        max_relationships: 3,
    }));
    let state = Arc::new(WebhookState {
        engine,
        allowed_users: vec![],
        bot_token: String::new(),
        sqlite,
        extractor,
        store,
        user_context: UserContext {
            user_id: String::new(),
            email: String::new(),
            display_name: String::new(),
        },
        min_occurrences: 0,
        gdrive_token: None,
        memory_store,
        session_manager,
        http,
        google_client_id,
        google_client_secret,
    });

    let governor_conf = Arc::new(
        tower_governor::governor::GovernorConfigBuilder::default()
            .per_millisecond(200)
            .burst_size(20)
            .finish()
            .expect("invalid governor config"),
    );

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/chat", post(chat_handler))
        .route("/api/auth/google/callback", get(oauth_callback_handler))
        .with_state(state)
        .layer(tower_governor::GovernorLayer {
            config: governor_conf,
        });

    let addr = format!("0.0.0.0:{port}");
    info!("HTTP-only server listening on {addr}");
    println!("trusty-telegram http-only server on port {port}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
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
    memory_store: Arc<MemoryStore>,
) -> Result<()> {
    // Retrieve or generate the webhook secret token (persisted across restarts).
    let webhook_secret = match sqlite.get_config("webhook_secret_token").ok().flatten() {
        Some(t) => t,
        None => {
            use rand::Rng;
            let t: String = rand::thread_rng()
                .sample_iter(&rand::distributions::Alphanumeric)
                .take(32)
                .map(char::from)
                .collect();
            sqlite.set_config("webhook_secret_token", &t)?;
            t
        }
    };

    // Register webhook with Telegram.
    let http = reqwest::Client::new();
    api_set_webhook(&http, &bot_token, &webhook_url, Some(&webhook_secret)).await?;

    let google_client_id = std::env::var("GOOGLE_CLIENT_ID").unwrap_or_default();
    let google_client_secret =
        trusty_core::secrets::get("GOOGLE_CLIENT_SECRET").unwrap_or_default();

    let session_manager = Arc::new(SessionManager::new(sqlite.clone()));
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
        memory_store,
        session_manager,
        http,
        google_client_id,
        google_client_secret,
    });

    let governor_conf = Arc::new(
        tower_governor::governor::GovernorConfigBuilder::default()
            .per_millisecond(200) // replenish 1 token per 200ms = 5 req/s
            .burst_size(20)
            .finish()
            .expect("invalid governor config"),
    );

    // ── Slack integration (optional) ────────────────────────────────────
    // Socket Mode (preferred for ELT rollout): when SLACK_APP_TOKEN is set,
    // start a persistent WebSocket connection to Slack — no public URL needed.
    //
    // HTTP webhook fallback: when only SLACK_BOT_TOKEN + SLACK_SIGNING_SECRET
    // are set (no SLACK_APP_TOKEN), mount /slack/events on this port so the
    // existing ngrok tunnel handles both Telegram and Slack.
    let slack_router: Option<axum::Router> = if let Some(ss) =
        trusty_slack::slack_state_from_env(Arc::clone(&state.engine), Arc::clone(&state.store))
    {
        if trusty_slack::spawn_socket_mode(Arc::clone(&ss)) {
            tracing::info!("Slack Socket Mode started (no public URL needed)");
            None
        } else {
            tracing::info!("Slack webhook mounted at /slack/events (HTTP mode)");
            Some(trusty_slack::build_slack_router(ss))
        }
    } else {
        None
    };

    // Build main router and finalize its state → Router<()>
    let main_app = Router::new()
        .route("/health", get(health_handler))
        .route("/chat", post(chat_handler))
        .route("/webhook/telegram", post(webhook_handler))
        .route("/api/auth/google/callback", get(oauth_callback_handler))
        .with_state(state);

    // Merge Slack sub-router (also Router<()>) then apply shared layers
    let app =
        main_app
            .merge(slack_router.unwrap_or_default())
            .layer(tower_governor::GovernorLayer {
                config: governor_conf,
            });

    let addr = format!("0.0.0.0:{port}");
    info!("Webhook server listening on {addr}");
    println!("trusty-telegram webhook server on port {port}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Long-poll mode (fallback)
// ---------------------------------------------------------------------------

async fn run_poll(
    bot_token: String,
    engine: Arc<ChatEngine>,
    allowed_users: Vec<i64>,
    session_manager: Arc<SessionManager>,
    memory_store: Arc<MemoryStore>,
    user_id: String,
) {
    info!("Starting Telegram bot long-polling");
    let bot = Bot::new(bot_token);

    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let engine = engine.clone();
        let allowed = allowed_users.clone();
        let session_manager = Arc::clone(&session_manager);
        let memory_store = Arc::clone(&memory_store);
        let user_id = user_id.clone();
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

            // Load persisted session by stable chat_id UUID (matches webhook behaviour).
            let session_uuid = uuid::Uuid::from_u128(msg.chat.id.0 as u128);
            let mut session = session_manager
                .load_or_create(session_uuid, 40)
                .await
                .unwrap_or_else(|_| SessionManager::new_session(&format!("tg_{}", msg.chat.id)));

            match engine.chat(&mut session, &text).await {
                Ok(response) => {
                    info!(
                        memories = response.memories_to_save.len(),
                        "Chat turn complete"
                    );

                    let reply = if response.reply.trim().is_empty() {
                        "👍".to_string()
                    } else {
                        response.reply.clone()
                    };
                    bot.send_message(msg.chat.id, &reply).await?;

                    // Persist session (best-effort).
                    let sm_save = Arc::clone(&session_manager);
                    let session_to_save = session.clone();
                    tokio::spawn(async move {
                        if let Err(e) = sm_save.save(&session_to_save).await {
                            warn!("poll: failed to save session: {e}");
                        }
                    });

                    // Persist memories (best-effort).
                    if !response.memories_to_save.is_empty() {
                        let mem_store = Arc::clone(&memory_store);
                        let memories = response.memories_to_save.clone();
                        let uid = user_id.clone();
                        tokio::spawn(async move {
                            for mem in memories {
                                if let Err(e) = mem_store
                                    .save(
                                        &uid,
                                        &mem.content,
                                        mem.category,
                                        mem.related_entities,
                                        mem.importance,
                                        None,
                                    )
                                    .await
                                {
                                    warn!("poll: failed to save memory: {e}");
                                }
                            }
                        });
                    }
                }
                Err(e) => {
                    let e_str = e.to_string();
                    error!("Chat error: {e_str}");
                    let user_msg = if e_str.contains("402")
                        || e_str.contains("Insufficient credits")
                    {
                        "⚠️ OpenRouter out of credits. Add more at openrouter.ai/settings/credits"
                            .to_string()
                    } else if e_str.contains("429")
                        || e_str.contains("rate limit")
                        || e_str.contains("Rate limit")
                    {
                        "⚠️ OpenRouter rate limit hit. Try again in a moment.".to_string()
                    } else if e_str.contains("500")
                        || e_str.contains("502")
                        || e_str.contains("503")
                        || e_str.contains("Internal Server Error")
                    {
                        "⚠️ OpenRouter service error (500). Try again in a moment.".to_string()
                    } else {
                        format!("⚠️ Error: {}", &e_str[..e_str.len().min(120)])
                    };
                    bot.send_message(msg.chat.id, user_msg).await?;
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
    // Load env file based on TRUSTY_ENV.
    // Dev: ~/.config/trusty-izzie-dev/config.env (fallback: .env in cwd)
    // Prod: ~/.config/trusty-izzie/config.env (fallback: .env in cwd)
    {
        let is_dev = std::env::var("TRUSTY_ENV")
            .map(|v| v == "dev")
            .unwrap_or(false);
        let home = std::env::var("HOME").unwrap_or_default();
        let config_env_path = if is_dev {
            std::path::PathBuf::from(&home).join(".config/trusty-izzie-dev/config.env")
        } else {
            std::path::PathBuf::from(&home).join(".config/trusty-izzie/config.env")
        };
        if config_env_path.exists() {
            dotenvy::from_path(&config_env_path).ok();
        } else {
            dotenvy::dotenv().ok();
        }
    }
    trusty_core::secrets::migrate_from_env();

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
        http_only: false,
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
            http_only,
        } => {
            // Prefer the explicitly paired token in SQLite over ambient env vars,
            // so that AI Commander's TELEGRAM_BOT_TOKEN doesn't bleed in.
            let token = sqlite
                .get_config("telegram_bot_token")
                .ok()
                .flatten()
                .or_else(|| trusty_core::secrets::get("TELEGRAM_BOT_TOKEN"))
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

            let instance_id = load_instance_id();
            let store = Arc::new(Store::open(&data_dir, &instance_id).await?);

            // Cache entity/memory counts in kv_config so get_izzie_status can report them.
            {
                let store_c = Arc::clone(&store);
                tokio::spawn(async move {
                    match store_c.count_vectors().await {
                        Ok((entities, memories)) => {
                            let _ = store_c
                                .sqlite
                                .set_config("entity_count", &entities.to_string());
                            let _ = store_c
                                .sqlite
                                .set_config("memory_count", &memories.to_string());
                            tracing::debug!(
                                entities,
                                memories,
                                "vector counts cached in kv_config"
                            );
                        }
                        Err(e) => tracing::warn!("failed to count vectors: {e}"),
                    }
                });
            }

            let embedder = Arc::new(
                Embedder::new(EmbeddingModel::AllMiniLmL6V2)
                    .map_err(|e| anyhow!("failed to init embedder: {e}"))?,
            );
            let memory_recaller = Arc::new(MemoryRecaller::new(
                Arc::clone(&store),
                Arc::clone(&embedder),
            ));
            let memory_store =
                Arc::new(MemoryStore::new(Arc::clone(&store), Arc::clone(&embedder)));
            let assembler = ContextAssembler::new(5, 10)
                .with_lance(Arc::clone(&store.lance))
                .with_memory_recaller(memory_recaller);

            let api_key = trusty_core::secrets::get("OPENROUTER_API_KEY").unwrap_or_default();

            // Build user context from environment / config.
            let primary_email = std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_default();
            let user_context = UserContext {
                user_id: instance_id.clone(),
                email: primary_email.clone(),
                display_name: primary_email.clone(),
            };

            let engine = Arc::new(
                ChatEngine::new_with_context(
                    config.openrouter.base_url.clone(),
                    api_key.clone(),
                    config.openrouter.chat_model.clone(),
                    config.chat.max_tool_iterations,
                    assembler,
                )
                .with_sqlite(Arc::clone(&store.sqlite))
                .with_agents_dir(data_dir.join("agents"))
                .with_skills_dir(config.agents.skills_dir.clone())
                .with_instance_label(config.instance.label.clone())
                .with_skills(vec![
                    std::sync::Arc::new(MetroNorthSkill),
                    std::sync::Arc::new(WeatherSkill),
                ]),
            );

            // Seed all configured Gmail accounts (idempotent).
            // TRUSTY_GMAIL_ACCOUNTS is a comma-separated list; falls back to TRUSTY_PRIMARY_EMAIL.
            let gmail_accounts: Vec<String> = {
                let from_env = std::env::var("TRUSTY_GMAIL_ACCOUNTS").unwrap_or_default();
                let mut list: Vec<String> = from_env
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if list.is_empty() && !primary_email.is_empty() {
                    list.push(primary_email.clone());
                }
                list
            };
            for (i, email) in gmail_accounts.iter().enumerate() {
                let account_type = if i == 0 { "primary" } else { "secondary" };
                if let Err(e) = store.sqlite.add_account(email, Some(email), account_type) {
                    // add_account uses INSERT OR IGNORE semantics; log but don't fail.
                    warn!("Failed to seed account {email}: {e}");
                }
            }
            // Legacy: also call seed_primary_account for backward compat with existing data.
            if let Err(e) = store.sqlite.seed_primary_account(&primary_email) {
                warn!("Failed to seed primary account: {e}");
            }

            // Build the entity extractor.
            let extractor = Arc::new(EntityExtractor::new(ExtractorConfig {
                base_url: config.openrouter.base_url.clone(),
                api_key: api_key.clone(),
                model: config.openrouter.extraction_model.clone(),
                max_tokens: 2048,
                confidence_threshold: 0.85,
                max_relationships: 3,
            }));

            let min_occurrences = 2u32;
            let gdrive_token = sqlite.get_config("google_access_token").ok().flatten();

            // Email sync is handled by trusty-daemon via the event queue.
            // Do not run it inline here.

            if allowed.is_empty() {
                println!("trusty-telegram starting (no user restriction)...");
            } else {
                println!("trusty-telegram starting (allowed users: {:?})...", allowed);
            }

            if http_only {
                // HTTP server only — no Telegram connection.
                run_http_only(port, engine, Arc::clone(&store.sqlite), store, memory_store).await?;
            } else if poll {
                let poll_session_manager = Arc::new(SessionManager::new(Arc::clone(&store.sqlite)));
                run_poll(
                    token,
                    engine,
                    allowed,
                    poll_session_manager,
                    Arc::clone(&memory_store),
                    instance_id.clone(),
                )
                .await;
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
                    memory_store,
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
                .or_else(|| trusty_core::secrets::get("TELEGRAM_BOT_TOKEN"))
                .ok_or_else(|| {
                    anyhow!("No bot token found. Run: trusty-telegram pair --token <TOKEN>")
                })?;

            let http = reqwest::Client::new();
            match action {
                WebhookAction::Set { url } => {
                    api_set_webhook(&http, &token, &url, None).await?;
                }
                WebhookAction::Clear => {
                    api_delete_webhook(&http, &token).await?;
                }
            }
        }
    }

    Ok(())
}
