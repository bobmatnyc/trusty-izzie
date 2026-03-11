//! Sends a morning briefing to the user via Telegram at 8am local time.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::{info, warn};
use trusty_core::error::TrustyError;
use trusty_email::auth::GoogleAuthClient;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::{SqliteStore, Store};

use super::{DispatchResult, EventHandler};
use crate::scheduling::next_time_of_day_ts;
use crate::telegram_push::send_telegram_push;

pub struct MorningBriefingHandler {
    openrouter_base: String,
    openrouter_api_key: String,
}

impl MorningBriefingHandler {
    pub fn new(openrouter_base: String, openrouter_api_key: String) -> Self {
        Self {
            openrouter_base,
            openrouter_api_key,
        }
    }
}

struct DailyContext {
    events: Vec<String>,
    tasks: Vec<String>,
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

async fn fetch_todays_context(sqlite: &SqliteStore) -> DailyContext {
    let accounts = match sqlite.list_accounts() {
        Ok(a) => a,
        Err(e) => {
            warn!("Could not list accounts for morning briefing: {e}");
            return DailyContext {
                events: vec![],
                tasks: vec![],
            };
        }
    };

    let active: Vec<_> = accounts.into_iter().filter(|a| a.is_active).collect();
    if active.is_empty() {
        warn!("No active accounts found for morning briefing");
        return DailyContext {
            events: vec![],
            tasks: vec![],
        };
    }

    let http = reqwest::Client::new();
    let mut all_events = Vec::new();
    let mut all_tasks = Vec::new();

    for account in &active {
        let access_token = match get_valid_token(sqlite, &account.email).await {
            Ok(t) => t,
            Err(e) => {
                warn!(
                    "Could not get OAuth token for {} in morning briefing: {e}",
                    account.email
                );
                continue;
            }
        };
        let tag = format!("[{}: {}]", account.identity, account.email);
        let events = fetch_calendar_events(&http, &access_token, &tag).await;
        let tasks = fetch_open_tasks(&http, &access_token, &tag).await;
        all_events.extend(events);
        all_tasks.extend(tasks);
    }

    DailyContext {
        events: all_events,
        tasks: all_tasks,
    }
}

async fn fetch_calendar_events(
    http: &reqwest::Client,
    access_token: &str,
    tag: &str,
) -> Vec<String> {
    let now = chrono::Utc::now();
    let time_min = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let time_max = (now + chrono::Duration::days(1))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();

    let url = format!(
        "https://www.googleapis.com/calendar/v3/calendars/primary/events\
         ?timeMin={}&timeMax={}&singleEvents=true&orderBy=startTime&maxResults=20",
        time_min, time_max
    );

    let events_json: serde_json::Value = match http
        .get(&url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| e.to_string())
    {
        Err(e) => {
            warn!("Calendar API request failed: {e}");
            return vec![];
        }
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(v) => v,
            Err(e) => {
                warn!("Calendar API parse failed: {e}");
                return vec![];
            }
        },
    };

    if events_json.get("error").is_some() {
        return vec![];
    }

    let items = match events_json["items"].as_array() {
        Some(a) if !a.is_empty() => a.clone(),
        _ => return vec![],
    };

    let mut lines = Vec::new();
    for item in &items {
        let summary = item["summary"].as_str().unwrap_or("(no title)");
        let start = item["start"]["dateTime"]
            .as_str()
            .or_else(|| item["start"]["date"].as_str())
            .unwrap_or("unknown time");
        let location = item["location"].as_str().unwrap_or("");
        let attendee_count = item["attendees"].as_array().map(|a| a.len()).unwrap_or(0);

        let mut line = format!("• {} — {}", start, summary);
        if !location.is_empty() {
            line.push_str(&format!(" @ {}", location));
        }
        if attendee_count > 1 {
            line.push_str(&format!(" ({} attendees)", attendee_count));
        }
        line.push_str(&format!(" {}", tag));
        lines.push(line);
    }
    lines
}

async fn fetch_open_tasks(http: &reqwest::Client, access_token: &str, tag: &str) -> Vec<String> {
    let lists_resp: serde_json::Value = match http
        .get("https://tasks.googleapis.com/tasks/v1/users/@me/lists")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| e.to_string())
    {
        Err(e) => {
            warn!("Tasks API lists request failed: {e}");
            return vec![];
        }
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(v) => v,
            Err(e) => {
                warn!("Tasks API lists parse failed: {e}");
                return vec![];
            }
        },
    };

    if lists_resp.get("error").is_some() {
        return vec![];
    }

    let list_items = match lists_resp["items"].as_array() {
        Some(items) if !items.is_empty() => items.clone(),
        _ => return vec![],
    };

    let mut all_tasks = Vec::new();
    for list in &list_items {
        let list_id = list["id"].as_str().unwrap_or("@default");
        let url = format!(
            "https://tasks.googleapis.com/tasks/v1/lists/{}/tasks?maxResults=100&showCompleted=false&showHidden=false",
            list_id
        );

        let tasks_resp: serde_json::Value =
            match http.get(&url).bearer_auth(access_token).send().await {
                Err(_) => continue,
                Ok(resp) => match resp.json::<serde_json::Value>().await {
                    Ok(v) => v,
                    Err(_) => continue,
                },
            };

        if tasks_resp.get("error").is_some() {
            continue;
        }

        let tasks = match tasks_resp["items"].as_array() {
            Some(t) if !t.is_empty() => t.clone(),
            _ => continue,
        };

        for task in &tasks {
            let title = task["title"].as_str().unwrap_or("(untitled)");
            let due = task["due"].as_str().unwrap_or("");
            let mut line = format!("- [ ] {}", title);
            if !due.is_empty() {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(due) {
                    line.push_str(&format!(" (due: {})", dt.format("%b %d")));
                }
            }
            line.push_str(&format!(" {}", tag));
            all_tasks.push(line);
        }
    }
    all_tasks
}

#[async_trait]
impl EventHandler for MorningBriefingHandler {
    fn event_type(&self) -> EventType {
        EventType::MorningBriefing
    }

    async fn handle(
        &self,
        _event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let enabled = store
            .sqlite
            .get_pref("morning_briefing_enabled")
            .unwrap_or(None)
            .unwrap_or_else(|| "true".to_string());
        if enabled != "true" {
            info!("MorningBriefing disabled by user pref");
            return Ok(schedule_next_morning());
        }

        let context = fetch_todays_context(&store.sqlite).await;

        let briefing = generate_briefing(&self.openrouter_base, &self.openrouter_api_key, &context)
            .await
            .unwrap_or_else(|_| "Good morning! Ready to help with your day.".to_string());

        send_telegram_push(&store.sqlite, &briefing).await?;
        info!("MorningBriefing sent");

        Ok(schedule_next_morning())
    }
}

fn schedule_next_morning() -> DispatchResult {
    DispatchResult::Chain(vec![(
        EventType::MorningBriefing,
        EventPayload::MorningBriefing {},
        next_time_of_day_ts(8, 0),
    )])
}

async fn generate_briefing(
    base: &str,
    key: &str,
    ctx: &DailyContext,
) -> Result<String, TrustyError> {
    let events_text = if ctx.events.is_empty() {
        "No events today".to_string()
    } else {
        ctx.events.join("\n")
    };

    let tasks_text = if ctx.tasks.is_empty() {
        "No open tasks".to_string()
    } else {
        ctx.tasks.join("\n")
    };

    let prompt = format!(
        "You are Izzie, a personal AI assistant. Generate a warm, personalized good morning \
briefing based on what's ahead today. 3-5 sentences max. Be friendly and helpful.\n\n\
Today's calendar (next 24h, all accounts):\n{}\n\nOpen tasks (all accounts):\n{}",
        events_text, tasks_text
    );

    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", key))
        .json(&serde_json::json!({
            "model": "anthropic/claude-haiku-4.5",
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": 300
        }))
        .send()
        .await
        .map_err(|e| TrustyError::Http(e.to_string()))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| TrustyError::Serialization(e.to_string()))?;
    Ok(json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("Good morning!")
        .to_string())
}
