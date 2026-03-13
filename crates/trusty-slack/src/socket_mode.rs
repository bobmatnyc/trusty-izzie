//! Slack Socket Mode client — replaces the HTTP webhook when SLACK_APP_TOKEN is set.
//!
//! Protocol:
//!  1. POST https://slack.com/api/apps.connections.open with the App-Level Token
//!     (xapp-...) to get a one-time WSS URL.
//!  2. Connect to that URL via WebSocket.
//!  3. Slack pushes event envelopes as JSON text frames.
//!  4. For each envelope we must ACK within 3 s: {"envelope_id": "<id>"}
//!  5. Reconnect automatically on disconnect.
//!
//! Event payloads are routed through the same `handler::dispatch_event` logic
//! used by the HTTP webhook path — zero duplication.

use std::sync::Arc;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::time::{sleep, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::{events::SlackPayload, handler::SlackState};

/// Slack Socket Mode envelope wrapper.
#[derive(Debug, Deserialize)]
struct Envelope {
    envelope_id: String,
    #[serde(rename = "type")]
    kind: String,
    payload: Option<serde_json::Value>,
    #[serde(default)]
    retry_attempt: u32,
}

/// Response from apps.connections.open
#[derive(Debug, Deserialize)]
struct ConnectionsOpenResponse {
    ok: bool,
    url: Option<String>,
    error: Option<String>,
}

/// Obtain a fresh WSS URL from the Slack API using the App-Level Token.
async fn get_wss_url(app_token: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let resp: ConnectionsOpenResponse = client
        .post("https://slack.com/api/apps.connections.open")
        .bearer_auth(app_token)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send()
        .await
        .context("apps.connections.open HTTP request failed")?
        .json()
        .await
        .context("failed to parse apps.connections.open response")?;

    if !resp.ok {
        anyhow::bail!(
            "apps.connections.open failed: {}",
            resp.error.unwrap_or_else(|| "unknown error".into())
        );
    }

    resp.url
        .ok_or_else(|| anyhow::anyhow!("apps.connections.open returned no URL"))
}

/// Run the Socket Mode event loop forever, reconnecting on disconnect.
/// Call this from a `tokio::spawn` task at startup when SLACK_APP_TOKEN is present.
pub async fn run(state: Arc<SlackState>, app_token: String) {
    let mut backoff_secs = 1u64;

    loop {
        match run_once(&state, &app_token).await {
            Ok(()) => {
                info!("Socket Mode connection closed cleanly — reconnecting immediately");
                backoff_secs = 1;
            }
            Err(e) => {
                error!("Socket Mode error: {e:#} — reconnecting in {backoff_secs}s");
                sleep(Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs * 2).min(60);
            }
        }
    }
}

/// One connection lifetime.
async fn run_once(state: &Arc<SlackState>, app_token: &str) -> Result<()> {
    let wss_url = get_wss_url(app_token).await?;
    info!("Socket Mode: connecting to Slack WSS");

    let (ws_stream, _) = connect_async(&wss_url)
        .await
        .context("WebSocket connect failed")?;

    info!("Socket Mode: connected");

    let (mut write, mut read) = ws_stream.split();

    while let Some(msg_result) = read.next().await {
        let msg = match msg_result {
            Ok(m) => m,
            Err(e) => {
                warn!("WebSocket recv error: {e}");
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                let envelope: Envelope = match serde_json::from_str(&text) {
                    Ok(e) => e,
                    Err(e) => {
                        warn!("Failed to parse envelope: {e}\nRaw: {text}");
                        continue;
                    }
                };

                // Always ACK first — Slack requires < 3 s.
                let ack = serde_json::json!({ "envelope_id": envelope.envelope_id }).to_string();
                if let Err(e) = write.send(Message::Text(ack)).await {
                    warn!("Failed to send ACK: {e}");
                }

                // Skip retries to avoid double-processing.
                if envelope.retry_attempt > 0 {
                    continue;
                }

                match envelope.kind.as_str() {
                    "hello" => info!("Socket Mode: received hello from Slack"),
                    "disconnect" => {
                        info!("Socket Mode: Slack requested disconnect — reconnecting");
                        break;
                    }
                    "events_api" => {
                        if let Some(payload_val) = envelope.payload {
                            dispatch_socket_event(Arc::clone(state), payload_val).await;
                        }
                    }
                    other => {
                        // interactive, slash_commands, etc. — not used yet
                        info!("Socket Mode: unhandled envelope type '{other}'");
                    }
                }
            }
            Message::Ping(data) => {
                let _ = write.send(Message::Pong(data)).await;
            }
            Message::Close(_) => {
                info!("Socket Mode: server closed connection");
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

/// Route an events_api payload through the same logic as the HTTP handler.
async fn dispatch_socket_event(state: Arc<SlackState>, payload: serde_json::Value) {
    // Wrap the inner payload so it matches the SlackPayload::EventCallback shape.
    let wrapped = serde_json::json!({
        "type": "event_callback",
        "event": payload.get("event").cloned().unwrap_or_default(),
        "team_id": payload.get("team_id"),
        "api_app_id": payload.get("api_app_id"),
        "event_id": payload.get("event_id"),
        "event_time": payload.get("event_time"),
    });

    let slack_payload: SlackPayload = match serde_json::from_value(wrapped) {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to parse socket event payload: {e}");
            return;
        }
    };

    if let SlackPayload::EventCallback(callback) = slack_payload {
        tokio::spawn(crate::handler::dispatch_event_pub(state, *callback));
    }
}
