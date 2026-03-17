//! Sends an evening briefing to the user via Telegram at a configurable local time.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::{info, warn};
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::{SqliteStore, Store};

use super::{DispatchResult, EventHandler};
use crate::handlers::morning_briefing::{fetch_open_tasks, get_valid_token};
use crate::scheduling::next_time_of_day_ts;
use crate::telegram_push::send_telegram_push;

pub struct EveningBriefingHandler {
    openrouter_base: String,
    openrouter_api_key: String,
}

impl EveningBriefingHandler {
    pub fn new(openrouter_base: String, openrouter_api_key: String) -> Self {
        Self {
            openrouter_base,
            openrouter_api_key,
        }
    }
}

struct EveningContext {
    tomorrow_events: Vec<String>,
    tasks: Vec<String>,
}

async fn fetch_evening_context(sqlite: &SqliteStore) -> EveningContext {
    let accounts = match sqlite.list_accounts() {
        Ok(a) => a,
        Err(e) => {
            warn!("Could not list accounts for evening briefing: {e}");
            return EveningContext {
                tomorrow_events: vec![],
                tasks: vec![],
            };
        }
    };

    let active: Vec<_> = accounts.into_iter().filter(|a| a.is_active).collect();
    if active.is_empty() {
        return EveningContext {
            tomorrow_events: vec![],
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
                    "Could not get OAuth token for {} in evening briefing: {e}",
                    account.email
                );
                continue;
            }
        };
        let tag = format!("[{}]", account.identity);
        let events = fetch_tomorrow_events(&http, &access_token, &tag).await;
        let tasks = fetch_open_tasks(&http, &access_token, &tag).await;
        all_events.extend(events);
        all_tasks.extend(tasks);
    }

    EveningContext {
        tomorrow_events: all_events,
        tasks: all_tasks,
    }
}

async fn fetch_tomorrow_events(
    http: &reqwest::Client,
    access_token: &str,
    tag: &str,
) -> Vec<String> {
    use chrono::{Local, LocalResult, TimeZone};

    let now_utc = chrono::Utc::now();

    // time_min = midnight tonight (start of tomorrow local)
    let tomorrow = Local::now()
        .date_naive()
        .succ_opt()
        .unwrap_or_else(|| Local::now().date_naive());
    let tom_midnight = tomorrow.and_hms_opt(0, 0, 0).unwrap();
    let time_min_utc = match Local.from_local_datetime(&tom_midnight) {
        LocalResult::Single(dt) => dt.with_timezone(&chrono::Utc),
        LocalResult::Ambiguous(dt, _) => dt.with_timezone(&chrono::Utc),
        LocalResult::None => now_utc + chrono::Duration::hours(2),
    };
    let time_min_str = time_min_utc.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    // time_max = end of tomorrow local (23:59:59)
    let tom_eod = tomorrow.and_hms_opt(23, 59, 59).unwrap();
    let time_max_utc = match Local.from_local_datetime(&tom_eod) {
        LocalResult::Single(dt) => dt.with_timezone(&chrono::Utc),
        LocalResult::Ambiguous(dt, _) => dt.with_timezone(&chrono::Utc),
        LocalResult::None => time_min_utc + chrono::Duration::hours(24),
    };
    let time_max_str = time_max_utc.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let url = format!(
        "https://www.googleapis.com/calendar/v3/calendars/primary/events\
         ?timeMin={}&timeMax={}&singleEvents=true&orderBy=startTime&maxResults=20",
        time_min_str, time_max_str
    );

    let events_json: serde_json::Value = match http
        .get(&url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| e.to_string())
    {
        Err(e) => {
            warn!("Calendar API request failed (evening): {e}");
            return vec![];
        }
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(v) => v,
            Err(e) => {
                warn!("Calendar API parse failed (evening): {e}");
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
        if item["status"].as_str() == Some("cancelled") {
            continue;
        }
        let summary = item["summary"].as_str().unwrap_or("(no title)");
        let start = item["start"]["dateTime"]
            .as_str()
            .or_else(|| item["start"]["date"].as_str())
            .unwrap_or("unknown time");
        let location = item["location"].as_str().unwrap_or("");

        let mut line = format!("• {} — {}", start, summary);
        if !location.is_empty() {
            line.push_str(&format!(" @ {}", location));
        }
        line.push_str(&format!(" {}", tag));
        lines.push(line);
    }
    lines
}

#[async_trait]
impl EventHandler for EveningBriefingHandler {
    fn event_type(&self) -> EventType {
        EventType::EveningBriefing
    }

    async fn handle(
        &self,
        _event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let enabled = store
            .sqlite
            .get_pref("evening_briefing_enabled")
            .unwrap_or(None)
            .unwrap_or_else(|| "true".to_string());
        if enabled != "true" {
            info!("EveningBriefing disabled by user pref");
            return Ok(schedule_next_evening(&store.sqlite));
        }

        let context = fetch_evening_context(&store.sqlite).await;

        let location = store
            .sqlite
            .get_config("user_current_location")
            .unwrap_or(None)
            .unwrap_or_default();

        let briefing = generate_evening_briefing(
            &self.openrouter_base,
            &self.openrouter_api_key,
            &context,
            &location,
        )
        .await
        .unwrap_or_else(|_| "End of day.".to_string());

        send_telegram_push(&store.sqlite, &briefing).await?;
        info!("EveningBriefing sent");

        Ok(schedule_next_evening(&store.sqlite))
    }
}

fn schedule_next_evening(sqlite: &SqliteStore) -> DispatchResult {
    let hour = sqlite
        .get_config("evening_briefing_hour")
        .unwrap_or(None)
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|&h| (0..=23).contains(&h))
        .unwrap_or(22) as u32;

    DispatchResult::Chain(vec![(
        EventType::EveningBriefing,
        EventPayload::EveningBriefing {},
        next_time_of_day_ts(hour, 0),
    )])
}

async fn generate_evening_briefing(
    base: &str,
    key: &str,
    ctx: &EveningContext,
    location: &str,
) -> Result<String, TrustyError> {
    use chrono::Local;

    let now = Local::now();
    let date_header = format!(
        "Today is {}, {} {}, {}. Current time: {} {}.\n",
        now.format("%A"),
        now.format("%B"),
        now.format("%-d"),
        now.format("%Y"),
        now.format("%H:%M"),
        now.format("%Z"),
    );

    let location_line = if location.is_empty() {
        String::new()
    } else {
        format!("User's current location: {}\n", location)
    };

    let events_text = if ctx.tomorrow_events.is_empty() {
        "No events tomorrow".to_string()
    } else {
        ctx.tomorrow_events.join("\n")
    };

    let tasks_text = if ctx.tasks.is_empty() {
        "No open tasks".to_string()
    } else {
        ctx.tasks.join("\n")
    };

    let prompt = format!(
        "{}{}\
Preview tomorrow's schedule and any reminders. Bullet points. \
Tone: dispassionate and factual. No pleasantries, no filler. \
Style: briefing officer, not a wellness app.\n\
Lead with the earliest event. If events have locations, mention them. \
Note anything that needs preparation (early start, materials, travel).\n\
2-4 items max.\n\n\
Tomorrow's schedule:\n{}\n\nOpen tasks:\n{}",
        date_header, location_line, events_text, tasks_text
    );

    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", key))
        .json(&serde_json::json!({
            "model": "anthropic/claude-haiku-4.5",
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": 200
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
        .unwrap_or("End of day.")
        .to_string())
}
