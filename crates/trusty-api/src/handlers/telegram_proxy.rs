//! Reverse proxy for Telegram webhook requests.
//!
//! ngrok tunnels to trusty-api on port 3456, but the Telegram bot
//! handler runs as a separate binary on port 3457. This handler
//! forwards POST /webhook/telegram to the Telegram binary.

use axum::{body::Bytes, http::StatusCode, response::IntoResponse};

/// Forward the raw webhook body to the Telegram handler on port 3457.
pub async fn proxy_webhook(body: Bytes) -> impl IntoResponse {
    let client = reqwest::Client::new();
    match client
        .post("http://127.0.0.1:3457/webhook/telegram")
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
    {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = resp.text().await.unwrap_or_default();
            tracing::debug!("proxied telegram webhook -> {status}");
            (status, body)
        }
        Err(e) => {
            tracing::warn!("telegram proxy failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                format!("telegram handler unreachable: {e}"),
            )
        }
    }
}
