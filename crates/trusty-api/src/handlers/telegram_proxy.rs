//! Reverse proxy for Telegram webhook requests.
//!
//! ngrok tunnels to trusty-api on port 3456, but the Telegram bot
//! handler runs as a separate binary on port 3457. This handler
//! forwards POST /webhook/telegram to the Telegram binary.

use axum::{
    body::Bytes,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

/// Forward the raw webhook body to the Telegram handler on port 3457.
pub async fn proxy_webhook(headers: HeaderMap, body: Bytes) -> impl IntoResponse {
    let client = reqwest::Client::new();
    let mut req = client
        .post("http://127.0.0.1:3457/webhook/telegram")
        .header("Content-Type", "application/json");

    // Forward the Telegram secret token header if present
    if let Some(secret) = headers.get("X-Telegram-Bot-Api-Secret-Token") {
        req = req.header("X-Telegram-Bot-Api-Secret-Token", secret.as_bytes());
    }

    match req.body(body).send().await {
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
