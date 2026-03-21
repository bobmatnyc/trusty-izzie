//! Sends a morning briefing to the user via Telegram at a configurable local time.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};
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
pub async fn get_valid_token(sqlite: &SqliteStore, user_id: &str) -> anyhow::Result<String> {
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
    let client_secret = trusty_core::secrets::get("GOOGLE_CLIENT_SECRET").unwrap_or_default();
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

/// Geocode `address` via Nominatim, then query OpenRouteService foot-walking directions
/// from `origin` to the geocoded destination. Returns a formatted string like "~12 min walk"
/// or `None` on any error or missing data.
async fn compute_travel_time(
    origin: &str,
    destination: &str,
    ors_api_key: &str,
    http: &reqwest::Client,
) -> Option<String> {
    // Step 1: geocode origin
    let origin_coords = geocode(origin, http).await?;
    // Step 2: geocode destination
    let dest_coords = geocode(destination, http).await?;

    // Step 3: ORS foot-walking directions (lon,lat order)
    let url = format!(
        "https://api.openrouteservice.org/v2/directions/foot-walking\
         ?api_key={}&start={},{}&end={},{}",
        ors_api_key, origin_coords.1, origin_coords.0, dest_coords.1, dest_coords.0,
    );

    let resp = http
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| debug!("ORS request failed for '{}': {e}", destination))
        .ok()?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| debug!("ORS parse failed for '{}': {e}", destination))
        .ok()?;

    let summary = &json["features"][0]["properties"]["summary"];
    let duration_secs = match summary["duration"].as_f64() {
        Some(d) => d,
        None => {
            debug!("ORS response missing duration for '{}'", destination);
            return None;
        }
    };

    let total_mins = (duration_secs / 60.0).round() as u64;
    let label = if total_mins >= 60 {
        let h = total_mins / 60;
        let m = total_mins % 60;
        if m == 0 {
            format!("~{}h walk", h)
        } else {
            format!("~{}h {}min walk", h, m)
        }
    } else {
        format!("~{} min walk", total_mins)
    };

    Some(label)
}

/// Geocode an address string to (lat, lon) via Nominatim.
async fn geocode(address: &str, http: &reqwest::Client) -> Option<(f64, f64)> {
    let encoded = urlencoding::encode(address);
    let url = format!(
        "https://nominatim.openstreetmap.org/search?format=json&limit=1&q={}",
        encoded
    );

    let resp = http
        .get(&url)
        .header("User-Agent", "trusty-izzie/1.0 (bobmatnyc@gmail.com)")
        .header("Accept-Language", "en")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| debug!("Nominatim request failed for '{}': {e}", address))
        .ok()?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| debug!("Nominatim parse failed for '{}': {e}", address))
        .ok()?;

    let first = json.as_array()?.first()?;
    let lat = first["lat"].as_str()?.parse::<f64>().ok()?;
    let lon = first["lon"].as_str()?.parse::<f64>().ok()?;
    Some((lat, lon))
}

async fn fetch_todays_context(
    sqlite: &SqliteStore,
    user_location: &str,
    ors_key: Option<&str>,
    maps_provider: &str,
) -> DailyContext {
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
        let tag = format!("[{}]", account.identity);
        let events = fetch_calendar_events(
            &http,
            &access_token,
            &tag,
            user_location,
            ors_key,
            maps_provider,
        )
        .await;
        let tasks = fetch_open_tasks(&http, &access_token, &tag).await;
        all_events.extend(events);
        all_tasks.extend(tasks);
    }

    DailyContext {
        events: all_events,
        tasks: all_tasks,
    }
}

/// Format a calendar datetime string (RFC3339 or date-only) into "Mon Mar 16, 9:00 AM".
fn format_event_dt(dt_str: &str) -> (String, bool) {
    // Try full datetime first
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(dt_str) {
        use chrono::TimeZone as _;
        let local = chrono::Local.from_utc_datetime(&dt.naive_utc());
        return (local.format("%a %b %-d, %-I:%M %p").to_string(), false);
    }
    // Date-only (all-day events)
    if let Ok(d) = chrono::NaiveDate::parse_from_str(dt_str, "%Y-%m-%d") {
        return (d.format("%a %b %-d").to_string(), true);
    }
    (dt_str.to_string(), false)
}

/// Strip HTML tags from a string, returning only text content.
fn strip_html_tags(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result.trim().to_string()
}

async fn fetch_calendar_events(
    http: &reqwest::Client,
    access_token: &str,
    tag: &str,
    user_location: &str,
    ors_key: Option<&str>,
    maps_provider: &str,
) -> Vec<String> {
    use chrono::{Local, LocalResult, TimeZone};

    // time_min = local midnight today
    let today = Local::now().date_naive();
    let midnight = today.and_hms_opt(0, 0, 0).unwrap();
    let time_min = match Local.from_local_datetime(&midnight) {
        LocalResult::Single(dt) => dt.with_timezone(&chrono::Utc),
        LocalResult::Ambiguous(dt, _) => dt.with_timezone(&chrono::Utc),
        LocalResult::None => chrono::Utc::now(),
    };
    // time_max = end of today (local midnight + 24h)
    let time_max = time_min + chrono::Duration::days(1);

    let time_min_str = time_min.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let time_max_str = time_max.format("%Y-%m-%dT%H:%M:%SZ").to_string();

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
        // Skip cancelled events
        if item["status"].as_str() == Some("cancelled") {
            continue;
        }

        let summary = item["summary"].as_str().unwrap_or("(no title)");
        let location = item["location"].as_str().unwrap_or("");
        let description = item["description"].as_str().unwrap_or("");
        let attendee_count = item["attendees"].as_array().map(|a| a.len()).unwrap_or(0);

        // Determine if all-day
        let start_dt_str = item["start"]["dateTime"].as_str();
        let start_date_str = item["start"]["date"].as_str();
        let end_dt_str = item["end"]["dateTime"].as_str();

        let line = if let Some(start_s) = start_dt_str {
            let (start_fmt, _) = format_event_dt(start_s);
            let end_part = end_dt_str
                .map(|e| {
                    let (ef, _) = format_event_dt(e);
                    // strip the date prefix — keep only time portion after ", "
                    ef.split_once(", ").map(|x| x.1).unwrap_or(&ef).to_string()
                })
                .unwrap_or_default();

            let mut l = if end_part.is_empty() {
                format!("• {} — {}", start_fmt, summary)
            } else {
                format!("• {}–{} — {}", start_fmt, end_part, summary)
            };
            if !location.is_empty() {
                l.push_str(&format!(
                    " @ {}",
                    trusty_core::maps::maps_link(location, maps_provider)
                ));
                if !user_location.is_empty() {
                    if let Some(key) = ors_key {
                        if let Some(travel) =
                            compute_travel_time(user_location, location, key, http).await
                        {
                            l.push_str(&format!(" (→ {})", travel));
                        }
                    }
                }
            }
            // Append description snippet (HTML-stripped, truncated to 200 chars)
            let desc_clean = strip_html_tags(description);
            if !desc_clean.is_empty() {
                let snippet = if desc_clean.len() > 200 {
                    format!("{}…", &desc_clean[..desc_clean.floor_char_boundary(200)])
                } else {
                    desc_clean
                };
                l.push_str(&format!(" | {}", snippet));
            }
            if attendee_count > 1 {
                l.push_str(&format!(" ({} attendees)", attendee_count));
            }
            l.push_str(&format!(" {}", tag));
            l
        } else if let Some(date_s) = start_date_str {
            let (date_fmt, _) = format_event_dt(date_s);
            let mut l = format!("• {}, All day — {}", date_fmt, summary);
            // Append description snippet for all-day events too
            let desc_clean = strip_html_tags(description);
            if !desc_clean.is_empty() {
                let snippet = if desc_clean.len() > 200 {
                    format!("{}…", &desc_clean[..desc_clean.floor_char_boundary(200)])
                } else {
                    desc_clean
                };
                l.push_str(&format!(" | {}", snippet));
            }
            l.push_str(&format!(" {}", tag));
            l
        } else {
            continue;
        };

        lines.push(line);
    }
    lines
}

pub async fn fetch_open_tasks(
    http: &reqwest::Client,
    access_token: &str,
    tag: &str,
) -> Vec<String> {
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
            return Ok(schedule_next_morning(&store.sqlite));
        }

        let location = store
            .sqlite
            .get_config("user_current_location")
            .unwrap_or(None)
            .unwrap_or_default();

        let ors_key = std::env::var("ORS_API_KEY").ok();
        let maps_provider = store
            .sqlite
            .get_config("maps_provider")
            .unwrap_or(None)
            .unwrap_or_else(|| "google".to_string());

        let context =
            fetch_todays_context(&store.sqlite, &location, ors_key.as_deref(), &maps_provider)
                .await;

        let briefing = generate_briefing(
            &self.openrouter_base,
            &self.openrouter_api_key,
            &context,
            &location,
        )
        .await
        .unwrap_or_else(|_| "Here are today's priorities.".to_string());

        send_telegram_push(&store.sqlite, &briefing).await?;
        info!("MorningBriefing sent");

        Ok(schedule_next_morning(&store.sqlite))
    }
}

fn schedule_next_morning(sqlite: &SqliteStore) -> DispatchResult {
    let hour = sqlite
        .get_config("morning_briefing_hour")
        .unwrap_or(None)
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|&h| (0..=23).contains(&h))
        .unwrap_or(7) as u32;

    DispatchResult::Chain(vec![(
        EventType::MorningBriefing,
        EventPayload::MorningBriefing {},
        next_time_of_day_ts(hour, 0),
    )])
}

/// Extract a city name from a location string using a simple heuristic:
/// take the last comma-separated component; if it looks like a ZIP code or
/// single-word country, take the second-to-last instead.
fn extract_city(location: &str) -> &str {
    let parts: Vec<&str> = location.split(',').collect();
    if parts.len() < 2 {
        return location.trim();
    }
    let last = parts[parts.len() - 1].trim();
    // If the last part is all digits/spaces (ZIP) or short uppercase (state/country abbreviation),
    // use the second-to-last component.
    let looks_like_zip_or_abbrev = last.chars().all(|c| c.is_ascii_digit() || c == ' ')
        || (last.len() <= 3 && last.chars().all(|c| c.is_ascii_alphabetic()));
    if looks_like_zip_or_abbrev && parts.len() >= 3 {
        parts[parts.len() - 2].trim()
    } else {
        last
    }
}

async fn generate_briefing(
    base: &str,
    key: &str,
    ctx: &DailyContext,
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

    // City-mismatch check: if events appear to be in a different city than stored location,
    // append a note so the LLM can ask the user for their hotel address.
    let city_mismatch_note = {
        // Find first non-empty event location by scanning ctx.events for " @ " marker.
        let event_location = ctx.events.iter().find_map(|line| {
            let at_pos = line.find(" @ ")?;
            let after = &line[at_pos + 3..];
            // Strip any trailing travel-time annotation " (→ ...)" or " (N attendees)"
            let end = after.find(" (").unwrap_or(after.len());
            let loc = after[..end].trim();
            if loc.is_empty() {
                None
            } else {
                Some(loc.to_string())
            }
        });

        match event_location {
            Some(ref ev_loc) if !location.is_empty() => {
                let event_city = extract_city(ev_loc);
                let user_city = extract_city(location);
                let event_city_lower = event_city.to_lowercase();
                let user_loc_lower = location.to_lowercase();
                let user_city_lower = user_city.to_lowercase();
                // Mismatch: event city not contained in user_location and vice versa.
                if !user_loc_lower.contains(&event_city_lower)
                    && !event_city_lower.contains(&user_city_lower)
                {
                    format!(
                        "\nNote: stored location is \"{}\" but events appear to be in {}. \
If the user is traveling, ask them for their hotel address.",
                        location, event_city
                    )
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        }
    };

    let prompt = format!(
        "{}{}\
Generate a morning briefing focused on today's schedule and any reminders. \
Bullet points. Tone: dispassionate and factual. No pleasantries, no filler. \
Style: briefing officer, not a wellness app.\n\
Lead with the earliest event. If events have locations, mention them. \
Note anything that needs preparation (early start, materials, travel).\n\
2-4 items max.\n\n\
Today's calendar:\n{}\n\nOpen tasks (all accounts):\n{}{}",
        date_header, location_line, events_text, tasks_text, city_mismatch_note
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
        .unwrap_or("Here are today's priorities.")
        .to_string())
}
