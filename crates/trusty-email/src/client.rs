//! Gmail REST API client.

use anyhow::{Context, Result};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use trusty_models::email::EmailMessage;

const GMAIL_BASE: &str = "https://gmail.googleapis.com/gmail/v1";

// ---------------------------------------------------------------------------
// Serde shapes for the Gmail API responses
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct HistoryListResponse {
    history: Option<Vec<HistoryRecord>>,
}

#[derive(Deserialize)]
struct HistoryRecord {
    messages: Option<Vec<HistoryMessageRef>>,
}

#[derive(Deserialize)]
struct HistoryMessageRef {
    id: String,
}

#[derive(Deserialize)]
struct GmailMessage {
    id: String,
    #[serde(rename = "threadId")]
    thread_id: Option<String>,
    #[serde(rename = "labelIds")]
    label_ids: Option<Vec<String>>,
    snippet: Option<String>,
    payload: Option<GmailPayload>,
}

#[derive(Deserialize)]
struct GmailPayload {
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
    headers: Option<Vec<GmailHeader>>,
    body: Option<GmailBody>,
    parts: Option<Vec<GmailPayload>>,
}

#[derive(Deserialize)]
struct GmailHeader {
    name: String,
    value: String,
}

#[derive(Deserialize)]
struct GmailBody {
    data: Option<String>,
}

#[derive(Deserialize)]
struct UserProfile {
    #[serde(rename = "emailAddress")]
    email_address: String,
    #[serde(rename = "historyId")]
    history_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Body extraction helper
// ---------------------------------------------------------------------------

/// Recursively search `payload` for `text/plain` and `text/html` parts.
/// Returns `(plain_text, html)`.
fn extract_body(payload: &GmailPayload) -> (Option<String>, Option<String>) {
    let mut plain: Option<String> = None;
    let mut html: Option<String> = None;

    // Check this node's body data if a mime_type is declared.
    if let Some(mime) = &payload.mime_type {
        if let Some(body) = &payload.body {
            if let Some(data) = &body.data {
                if let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(data) {
                    if let Ok(text) = String::from_utf8(bytes) {
                        if mime == "text/plain" {
                            plain = Some(text);
                        } else if mime == "text/html" {
                            html = Some(text);
                        }
                    }
                }
            }
        }
    }

    // Recurse into child parts.
    if let Some(parts) = &payload.parts {
        for part in parts {
            let (p, h) = extract_body(part);
            if plain.is_none() {
                plain = p;
            }
            if html.is_none() {
                html = h;
            }
        }
    }

    (plain, html)
}

/// Extract the value of a named header (case-insensitive) from the list.
fn header_value(headers: &[GmailHeader], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.clone())
}

/// Split a comma-separated address header into individual addresses.
fn split_addresses(value: Option<String>) -> Vec<String> {
    match value {
        None => vec![],
        Some(s) if s.trim().is_empty() => vec![],
        Some(s) => s.split(',').map(|a| a.trim().to_string()).collect(),
    }
}

/// Parse the RFC 2822 `Date` header into a `DateTime<Utc>`.
///
/// Falls back to `Utc::now()` if the header is absent or unparseable.
fn parse_date(value: Option<String>) -> DateTime<Utc> {
    value
        .and_then(|s| {
            // Try RFC 2822 first, then a fallback format.
            DateTime::parse_from_rfc2822(&s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        })
        .unwrap_or_else(Utc::now)
}

// ---------------------------------------------------------------------------
// Public client
// ---------------------------------------------------------------------------

/// Wraps the Gmail API with typed methods.
pub struct GmailClient {
    http: reqwest::Client,
    access_token: String,
}

impl GmailClient {
    /// Construct from a valid access token.
    pub fn new(access_token: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");
        Self { http, access_token }
    }

    /// Build the `Authorization: Bearer …` header value.
    fn bearer(&self) -> String {
        format!("Bearer {}", self.access_token)
    }

    /// Fetch the list of SENT message IDs modified since `history_id`.
    ///
    /// Uses the Gmail History API for incremental polling.
    pub async fn list_history_since(&self, history_id: &str) -> Result<Vec<String>> {
        let url = format!("{GMAIL_BASE}/users/me/history");
        let resp: HistoryListResponse = self
            .http
            .get(&url)
            .header("Authorization", self.bearer())
            .query(&[
                ("historyTypes", "messageAdded"),
                ("labelId", "SENT"),
                ("startHistoryId", history_id),
            ])
            .send()
            .await
            .context("GET /history request failed")?
            .error_for_status()
            .context("GET /history returned error status")?
            .json()
            .await
            .context("failed to deserialise history list response")?;

        let ids: Vec<String> = resp
            .history
            .unwrap_or_default()
            .into_iter()
            .flat_map(|record| record.messages.unwrap_or_default())
            .map(|m| m.id)
            // Deduplicate while preserving order.
            .fold(Vec::new(), |mut acc, id| {
                if !acc.contains(&id) {
                    acc.push(id);
                }
                acc
            });

        Ok(ids)
    }

    /// Fetch and decode a single message by ID.
    pub async fn get_message(&self, message_id: &str) -> Result<EmailMessage> {
        let url = format!("{GMAIL_BASE}/users/me/messages/{message_id}");
        let raw: GmailMessage = self
            .http
            .get(&url)
            .header("Authorization", self.bearer())
            .query(&[("format", "full")])
            .send()
            .await
            .context("GET /messages/{id} request failed")?
            .error_for_status()
            .context("GET /messages/{id} returned error status")?
            .json()
            .await
            .context("failed to deserialise Gmail message")?;

        let payload = raw.payload.as_ref();
        let headers: &[GmailHeader] = payload.and_then(|p| p.headers.as_deref()).unwrap_or(&[]);

        let from = header_value(headers, "From").unwrap_or_default();
        let to = split_addresses(header_value(headers, "To"));
        let cc = split_addresses(header_value(headers, "Cc"));
        let bcc = split_addresses(header_value(headers, "Bcc"));
        let subject = header_value(headers, "Subject").unwrap_or_default();
        let date = parse_date(header_value(headers, "Date"));

        let (body_text, body_html) = payload.map(extract_body).unwrap_or((None, None));

        Ok(EmailMessage {
            id: raw.id,
            thread_id: raw.thread_id.unwrap_or_default(),
            from,
            to,
            cc,
            bcc,
            subject,
            body_text,
            body_html,
            snippet: raw.snippet,
            labels: raw.label_ids.unwrap_or_default(),
            date,
            is_sent: true,
            ingested_at: Utc::now(),
        })
    }

    /// Return the authenticated user's email address and display name.
    ///
    /// Gmail's `/profile` endpoint does not expose a display name, so the
    /// email address is returned for both fields.
    pub async fn get_user_profile(&self) -> Result<(String, String)> {
        let profile = self.fetch_profile().await?;
        Ok((profile.email_address.clone(), profile.email_address))
    }

    /// Return the current Gmail `historyId` for the authenticated mailbox.
    pub async fn get_history_id(&self) -> Result<String> {
        let profile = self.fetch_profile().await?;
        profile
            .history_id
            .ok_or_else(|| anyhow::anyhow!("historyId missing from profile response"))
    }

    /// Shared helper: fetch the `/profile` resource once.
    async fn fetch_profile(&self) -> Result<UserProfile> {
        let url = format!("{GMAIL_BASE}/users/me/profile");
        self.http
            .get(&url)
            .header("Authorization", self.bearer())
            .send()
            .await
            .context("GET /profile request failed")?
            .error_for_status()
            .context("GET /profile returned error status")?
            .json()
            .await
            .context("failed to deserialise user profile")
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that header extraction and address splitting work on a realistic
    /// JSON payload — no network call required.
    #[test]
    fn test_parse_gmail_headers() {
        let json = r#"{
            "id": "msg001",
            "threadId": "thread001",
            "labelIds": ["SENT"],
            "snippet": "Hello world",
            "payload": {
                "mimeType": "text/plain",
                "headers": [
                    {"name": "From", "value": "Alice <alice@example.com>"},
                    {"name": "To", "value": "Bob <bob@example.com>, Carol <carol@example.com>"},
                    {"name": "Subject", "value": "Test Subject"},
                    {"name": "Date", "value": "Mon, 01 Jan 2024 12:00:00 +0000"}
                ],
                "body": {"data": "SGVsbG8gd29ybGQ"}
            }
        }"#;

        let raw: GmailMessage = serde_json::from_str(json).expect("valid JSON");
        let payload = raw.payload.as_ref().expect("payload present");
        let headers = payload.headers.as_deref().unwrap_or(&[]);

        assert_eq!(
            header_value(headers, "From").as_deref(),
            Some("Alice <alice@example.com>")
        );
        assert_eq!(
            header_value(headers, "Subject").as_deref(),
            Some("Test Subject")
        );

        let to = split_addresses(header_value(headers, "To"));
        assert_eq!(to.len(), 2);
        assert_eq!(to[0], "Bob <bob@example.com>");
        assert_eq!(to[1], "Carol <carol@example.com>");
    }

    /// Verify that base64url-encoded body data decodes to the expected string.
    #[test]
    fn test_base64_body_decoding() {
        // "Hello world" encoded as base64url (no padding).
        let encoded = "SGVsbG8gd29ybGQ";
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .expect("valid base64url");
        let text = String::from_utf8(bytes).expect("valid UTF-8");
        assert_eq!(text, "Hello world");
    }

    /// Verify recursive MIME part extraction picks up plain and HTML parts.
    #[test]
    fn test_extract_body_multipart() {
        // "plain text" → base64url = "cGxhaW4gdGV4dA"
        // "<b>html</b>" → base64url = "PGI-aHRtbDwvYj4"  (URL_SAFE_NO_PAD)
        let plain_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("plain text");
        let html_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("<b>html</b>");

        let json = format!(
            r#"{{
                "mimeType": "multipart/alternative",
                "parts": [
                    {{
                        "mimeType": "text/plain",
                        "body": {{"data": "{plain_b64}"}}
                    }},
                    {{
                        "mimeType": "text/html",
                        "body": {{"data": "{html_b64}"}}
                    }}
                ]
            }}"#
        );

        let payload: GmailPayload = serde_json::from_str(&json).expect("valid JSON");
        let (plain, html) = extract_body(&payload);
        assert_eq!(plain.as_deref(), Some("plain text"));
        assert_eq!(html.as_deref(), Some("<b>html</b>"));
    }

    /// Verify header_value is case-insensitive.
    #[test]
    fn test_header_value_case_insensitive() {
        let headers = vec![GmailHeader {
            name: "content-type".to_string(),
            value: "text/plain".to_string(),
        }];
        assert_eq!(
            header_value(&headers, "Content-Type").as_deref(),
            Some("text/plain")
        );
    }

    /// Verify empty / whitespace address fields are handled gracefully.
    #[test]
    fn test_split_addresses_empty() {
        assert!(split_addresses(None).is_empty());
        assert!(split_addresses(Some(String::new())).is_empty());
        assert!(split_addresses(Some("  ".to_string())).is_empty());
    }

    /// Verify RFC 2822 date parsing.
    #[test]
    fn test_parse_date_rfc2822() {
        use chrono::Datelike as _;
        let dt = parse_date(Some("Mon, 01 Jan 2024 12:00:00 +0000".to_string()));
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 1);
    }
}
