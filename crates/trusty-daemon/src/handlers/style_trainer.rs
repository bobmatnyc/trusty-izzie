//! StyleTrainerHandler — analyzes sent email snippets to build a writing style profile.
//!
//! Runs monthly. Reads the most recent sent emails across all Google accounts,
//! calls Sonnet to extract style characteristics, and stores the result in
//! kv_config as `communication_style_work` and `communication_style_personal`.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::{info, warn};
use trusty_core::error::TrustyError;
use trusty_email::auth::GoogleAuthClient;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::{SqliteStore, Store};

use super::{DispatchResult, EventHandler};

pub struct StyleTrainerHandler {
    openrouter_base: String,
    openrouter_api_key: String,
}

impl StyleTrainerHandler {
    pub fn new(openrouter_base: String, openrouter_api_key: String) -> Self {
        Self {
            openrouter_base,
            openrouter_api_key,
        }
    }
}

/// Consumer email domains considered "personal".
fn is_personal_domain(email: &str) -> bool {
    let lower = email.to_lowercase();
    [
        "@gmail.com",
        "@yahoo.com",
        "@hotmail.com",
        "@outlook.com",
        "@icloud.com",
        "@me.com",
    ]
    .iter()
    .any(|d| lower.ends_with(d))
}

/// Return a valid (non-expired) access token for `user_id`, refreshing if needed.
async fn get_valid_token(sqlite: &SqliteStore, user_id: &str) -> anyhow::Result<String> {
    let token = sqlite
        .get_oauth_token(user_id)?
        .ok_or_else(|| anyhow::anyhow!("No OAuth token stored for {}", user_id))?;

    let needs_refresh = token
        .expires_at
        .map(|exp| exp - chrono::Utc::now().timestamp() < 300)
        .unwrap_or(false);

    if !needs_refresh {
        return Ok(token.access_token);
    }

    let refresh_token = token
        .refresh_token
        .ok_or_else(|| anyhow::anyhow!("No refresh token for {}; re-auth required", user_id))?;

    let client_id = std::env::var("GOOGLE_CLIENT_ID").unwrap_or_default();
    let client_secret = std::env::var("GOOGLE_CLIENT_SECRET").unwrap_or_default();
    let ngrok =
        std::env::var("TRUSTY_NGROK_DOMAIN").unwrap_or_else(|_| "izzie.ngrok.dev".to_string());
    let redirect_uri = format!("https://{}/api/auth/google/callback", ngrok);

    let auth = GoogleAuthClient::new(client_id, client_secret, redirect_uri);
    let new_token = auth.refresh_token(&refresh_token).await?;
    let new_expires_at = Some(chrono::Utc::now().timestamp() + new_token.expires_in as i64);

    sqlite.refresh_oauth_token(
        user_id,
        &new_token.access_token,
        new_token.refresh_token.as_deref(),
        new_expires_at,
    )?;

    Ok(new_token.access_token)
}

/// Fetch the last `max_results` sent email snippets for an account.
/// Returns list of (snippet, to_header) pairs.
async fn fetch_sent_snippets(
    http: &reqwest::Client,
    access_token: &str,
    max_results: u32,
) -> Vec<(String, String)> {
    let list_url = format!(
        "https://gmail.googleapis.com/gmail/v1/users/me/messages?labelIds=SENT&maxResults={}",
        max_results
    );

    let list_json: serde_json::Value =
        match http.get(&list_url).bearer_auth(access_token).send().await {
            Err(e) => {
                warn!("Gmail SENT list request failed: {e}");
                return vec![];
            }
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(v) => v,
                Err(e) => {
                    warn!("Gmail SENT list parse failed: {e}");
                    return vec![];
                }
            },
        };

    if list_json.get("error").is_some() {
        warn!("Gmail SENT list error: {}", list_json["error"]);
        return vec![];
    }

    let messages = match list_json["messages"].as_array() {
        Some(m) if !m.is_empty() => m.clone(),
        _ => return vec![],
    };

    let mut snippets = Vec::new();
    for msg in &messages {
        let id = match msg["id"].as_str() {
            Some(id) => id,
            None => continue,
        };

        let msg_url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}?format=metadata&metadataHeaders=Subject,To",
            id
        );

        let msg_json: serde_json::Value =
            match http.get(&msg_url).bearer_auth(access_token).send().await {
                Err(_) => continue,
                Ok(resp) => match resp.json::<serde_json::Value>().await {
                    Ok(v) => v,
                    Err(_) => continue,
                },
            };

        if msg_json.get("error").is_some() {
            continue;
        }

        let snippet = msg_json["snippet"].as_str().unwrap_or("").to_string();
        if snippet.is_empty() {
            continue;
        }

        // Extract To header
        let to_header = msg_json["payload"]["headers"]
            .as_array()
            .and_then(|headers| {
                headers.iter().find(|h| {
                    h["name"]
                        .as_str()
                        .map(|n| n.eq_ignore_ascii_case("To"))
                        .unwrap_or(false)
                })
            })
            .and_then(|h| h["value"].as_str())
            .unwrap_or("")
            .to_string();

        snippets.push((snippet, to_header));
    }

    snippets
}

/// Call OpenRouter/Sonnet to produce a style JSON profile from email snippets.
async fn analyze_style(
    base: &str,
    key: &str,
    snippets: &[(String, String)],
    label: &str,
) -> Option<String> {
    if snippets.is_empty() {
        return None;
    }

    let email_list = snippets
        .iter()
        .enumerate()
        .map(|(i, (snippet, to))| format!("{}. To: {}\nSnippet: {}", i + 1, to, snippet))
        .collect::<Vec<_>>()
        .join("\n\n");

    let user_prompt = format!(
        "Here are {} email snippets this person wrote ({} emails). Analyze their writing style.\n\n{}\n\nReturn JSON with these fields:\n{{\n  \"formality\": \"formal|semi-formal|casual\",\n  \"avg_sentence_length\": \"short|medium|long\",\n  \"greeting_style\": \"example greeting they use\",\n  \"closing_style\": \"example closing they use\",\n  \"tone\": \"direct|warm|analytical|collaborative\",\n  \"bullet_points\": true/false,\n  \"emoji_usage\": \"none|rare|moderate|frequent\",\n  \"signature_phrases\": [\"phrase1\", \"phrase2\"],\n  \"notification_template\": \"A 1-sentence template for sending them a notification that matches their style. Use {{message}} as the placeholder for content.\",\n  \"summary\": \"2-sentence plain English description of their writing style\"\n}}",
        snippets.len(),
        label,
        email_list
    );

    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let resp = match client
        .post(&url)
        .header("Authorization", format!("Bearer {}", key))
        .json(&serde_json::json!({
            "model": "anthropic/claude-sonnet-4-5",
            "messages": [
                {
                    "role": "system",
                    "content": "You are analyzing someone's email writing style. Extract a concise style profile as JSON.\nOutput ONLY valid JSON, no preamble."
                },
                {
                    "role": "user",
                    "content": user_prompt
                }
            ],
            "max_tokens": 600
        }))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!("StyleTrainer OpenRouter request failed: {e}");
            return None;
        }
    };

    let json: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            warn!("StyleTrainer OpenRouter parse failed: {e}");
            return None;
        }
    };

    json["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
}

#[async_trait]
impl EventHandler for StyleTrainerHandler {
    fn event_type(&self) -> EventType {
        EventType::StyleTraining
    }

    async fn handle(
        &self,
        _event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        info!("StyleTrainer: starting style analysis");

        let accounts = match store.sqlite.list_accounts() {
            Ok(a) => a,
            Err(e) => {
                warn!("StyleTrainer: could not list accounts: {e}");
                return Ok(schedule_next_training());
            }
        };

        let active: Vec<_> = accounts.into_iter().filter(|a| a.is_active).collect();
        if active.is_empty() {
            warn!("StyleTrainer: no active accounts");
            return Ok(schedule_next_training());
        }

        let http = reqwest::Client::new();
        let mut work_snippets: Vec<(String, String)> = Vec::new();
        let mut personal_snippets: Vec<(String, String)> = Vec::new();

        for account in &active {
            let access_token = match get_valid_token(&store.sqlite, &account.email).await {
                Ok(t) => t,
                Err(e) => {
                    warn!(
                        "StyleTrainer: could not get token for {}: {e}",
                        account.email
                    );
                    continue;
                }
            };

            let snippets = fetch_sent_snippets(&http, &access_token, 30).await;

            for (snippet, to) in snippets {
                // Classify by recipient domain
                if is_personal_domain(&to) {
                    personal_snippets.push((snippet, to));
                } else {
                    work_snippets.push((snippet, to));
                }
            }
        }

        info!(
            "StyleTrainer: {} work snippets, {} personal snippets",
            work_snippets.len(),
            personal_snippets.len()
        );

        // Analyze work style
        if let Some(work_style) = analyze_style(
            &self.openrouter_base,
            &self.openrouter_api_key,
            &work_snippets,
            "work",
        )
        .await
        {
            let sqlite = store.sqlite.clone();
            let style = work_style.clone();
            tokio::task::spawn_blocking(move || {
                sqlite.set_config("communication_style_work", &style)
            })
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;
            info!("StyleTrainer: work style saved");
        }

        // Analyze personal style
        if let Some(personal_style) = analyze_style(
            &self.openrouter_base,
            &self.openrouter_api_key,
            &personal_snippets,
            "personal",
        )
        .await
        {
            let sqlite = store.sqlite.clone();
            let style = personal_style.clone();
            tokio::task::spawn_blocking(move || {
                sqlite.set_config("communication_style_personal", &style)
            })
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;
            info!("StyleTrainer: personal style saved");
        }

        // Persist updated_at timestamp
        let updated_at = chrono::Utc::now().to_rfc3339();
        let sqlite = store.sqlite.clone();
        tokio::task::spawn_blocking(move || {
            sqlite.set_config("communication_style_updated_at", &updated_at)
        })
        .await
        .map_err(|e| TrustyError::Storage(e.to_string()))?
        .map_err(|e| TrustyError::Storage(e.to_string()))?;

        info!("StyleTrainer: complete");
        Ok(schedule_next_training())
    }
}

/// Reschedule 30 days out.
fn schedule_next_training() -> DispatchResult {
    let next = chrono::Utc::now().timestamp() + 30 * 24 * 3600;
    DispatchResult::Chain(vec![(
        EventType::StyleTraining,
        EventPayload::StyleTraining {},
        next,
    )])
}
