//! Slack request signature verification.
//!
//! Slack signs every webhook with HMAC-SHA256 over:
//!   `v0:{timestamp}:{raw_body}`
//! The signature is in the `X-Slack-Signature` header as `v0=<hex>`.
//! The timestamp is in `X-Slack-Request-Timestamp` (Unix seconds).
//!
//! We also reject requests where the timestamp is >5 minutes old
//! to protect against replay attacks.

use anyhow::{bail, Result};
use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Verify a Slack webhook request.
///
/// - `signing_secret` — `SLACK_SIGNING_SECRET` env var
/// - `timestamp`      — value of `X-Slack-Request-Timestamp` header
/// - `signature`      — value of `X-Slack-Signature` header
/// - `body`           — raw request body bytes
pub fn verify(signing_secret: &str, timestamp: &str, signature: &str, body: &[u8]) -> Result<()> {
    // Replay protection: reject if >5 minutes old.
    let ts: i64 = timestamp
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid timestamp"))?;
    let now = Utc::now().timestamp();
    if (now - ts).abs() > 300 {
        bail!("timestamp too old or too far in the future");
    }

    // Build the base string.
    let sig_base = format!(
        "v0:{}:{}",
        timestamp,
        std::str::from_utf8(body).unwrap_or_default()
    );

    // Compute HMAC-SHA256.
    let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes())
        .map_err(|e| anyhow::anyhow!("hmac init: {e}"))?;
    mac.update(sig_base.as_bytes());
    let computed = hex::encode(mac.finalize().into_bytes());
    let expected = format!("v0={computed}");

    // Constant-time comparison.
    if !constant_time_eq(signature.as_bytes(), expected.as_bytes()) {
        bail!("signature mismatch");
    }
    Ok(())
}

/// Constant-time byte comparison (avoids timing side-channels).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}
