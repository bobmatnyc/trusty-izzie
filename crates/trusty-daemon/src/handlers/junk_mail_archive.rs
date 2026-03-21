//! Proactive junk-mail cleanup — runs user-configurable inbox filter rules hourly.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::{InboxRule, Store};

use super::{DispatchResult, EventHandler};
use crate::handlers::morning_briefing::get_valid_token;
use crate::scheduling::next_interval_ts;
use crate::telegram_push::send_telegram_push;

pub struct JunkMailArchiveHandler;

/// Per-rule result counts for reporting.
struct RuleStats {
    count: usize,
    action: String,
}

/// Per-account results keyed by rule name.
type ArchiveStats = HashMap<String, RuleStats>;

/// List message IDs matching `query` for the authenticated user (max 100).
async fn list_message_ids_for_query(
    http: &reqwest::Client,
    access_token: &str,
    query: &str,
) -> anyhow::Result<Vec<String>> {
    let query_enc = urlencoding::encode(query);
    let url = format!(
        "https://gmail.googleapis.com/gmail/v1/users/me/messages?q={}&maxResults=100",
        query_enc
    );

    let resp: serde_json::Value = http
        .get(&url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Gmail list request failed: {e}"))?
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Gmail list parse failed: {e}"))?;

    if let Some(err) = resp.get("error") {
        return Err(anyhow::anyhow!("Gmail API error: {err}"));
    }

    let ids = resp["messages"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    Ok(ids)
}

/// Modify a message: remove `remove_labels`, add `add_labels`.
async fn modify_message(
    http: &reqwest::Client,
    access_token: &str,
    message_id: &str,
    remove_label_ids: &[&str],
    add_label_ids: &[&str],
) -> bool {
    let url = format!(
        "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}/modify",
        message_id
    );

    let body = serde_json::json!({
        "removeLabelIds": remove_label_ids,
        "addLabelIds": add_label_ids,
    });

    match http
        .post(&url)
        .bearer_auth(access_token)
        .json(&body)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => true,
        Ok(resp) => {
            warn!("Gmail modify {message_id} returned {}", resp.status());
            false
        }
        Err(e) => {
            warn!("Gmail modify request failed for {message_id}: {e}");
            false
        }
    }
}

/// Process one account against all enabled rules. Returns per-rule stats.
async fn process_account(
    http: &reqwest::Client,
    access_token: &str,
    account_email: &str,
    rules: &[InboxRule],
) -> ArchiveStats {
    let mut stats: ArchiveStats = HashMap::new();

    for rule in rules.iter().filter(|r| r.enabled) {
        let ids = match list_message_ids_for_query(http, access_token, &rule.gmail_query).await {
            Ok(ids) => ids,
            Err(e) => {
                warn!(
                    "JunkMailArchive: rule '{}' failed to list messages for {account_email}: {e}",
                    rule.name
                );
                continue;
            }
        };

        let mut count = 0usize;
        for id in &ids {
            let applied = match rule.action.as_str() {
                "trash" => modify_message(http, access_token, id, &["INBOX"], &["TRASH"]).await,
                "label" => {
                    if let Some(label) = rule.action_label.as_deref() {
                        modify_message(http, access_token, id, &[], &[label]).await
                    } else {
                        false
                    }
                }
                _ => {
                    // default: archive (remove INBOX)
                    modify_message(http, access_token, id, &["INBOX"], &[]).await
                }
            };
            if applied {
                count += 1;
            }
        }

        stats.insert(
            rule.name.clone(),
            RuleStats {
                count,
                action: rule.action.clone(),
            },
        );
    }

    stats
}

fn schedule_next_archive(sqlite: &trusty_store::SqliteStore) -> DispatchResult {
    let interval_minutes = sqlite
        .get_config("junk_mail_archive_interval_minutes")
        .unwrap_or(None)
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|&m| (10..=1440).contains(&m))
        .unwrap_or(60); // default: every 60 minutes

    DispatchResult::Chain(vec![(
        EventType::JunkMailArchive,
        EventPayload::JunkMailArchive {},
        next_interval_ts(interval_minutes),
    )])
}

#[async_trait]
impl EventHandler for JunkMailArchiveHandler {
    fn event_type(&self) -> EventType {
        EventType::JunkMailArchive
    }

    async fn handle(
        &self,
        _event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let accounts = match store.sqlite.list_accounts() {
            Ok(a) => a,
            Err(e) => {
                warn!("JunkMailArchive: could not list accounts: {e}");
                return Ok(schedule_next_archive(&store.sqlite));
            }
        };

        let active: Vec<_> = accounts.into_iter().filter(|a| a.is_active).collect();
        if active.is_empty() {
            info!("JunkMailArchive: no active accounts");
            return Ok(schedule_next_archive(&store.sqlite));
        }

        let rules = match store.sqlite.list_inbox_rules() {
            Ok(r) => r,
            Err(e) => {
                warn!("JunkMailArchive: could not load inbox rules: {e}");
                return Ok(schedule_next_archive(&store.sqlite));
            }
        };

        let enabled_rules: Vec<_> = rules.iter().filter(|r| r.enabled).collect();
        if enabled_rules.is_empty() {
            info!("JunkMailArchive: no enabled inbox rules");
            return Ok(schedule_next_archive(&store.sqlite));
        }

        let http = reqwest::Client::new();
        // Aggregate counts across all accounts, keyed by rule name.
        let mut totals: HashMap<String, RuleStats> = HashMap::new();

        for account in &active {
            let access_token = match get_valid_token(&store.sqlite, &account.email).await {
                Ok(t) => t,
                Err(e) => {
                    warn!(
                        "JunkMailArchive: could not get token for {}: {e}",
                        account.email
                    );
                    continue;
                }
            };

            let stats = process_account(&http, &access_token, &account.email, &rules).await;
            info!(
                "JunkMailArchive: processed {} rules for {}",
                stats.len(),
                account.email
            );
            for (name, rs) in stats {
                let entry = totals.entry(name).or_insert(RuleStats {
                    count: 0,
                    action: rs.action.clone(),
                });
                entry.count += rs.count;
            }
        }

        // Build notification sorted by rule name.
        let any_action = totals.values().any(|rs| rs.count > 0);
        let message = if any_action {
            let mut lines = vec!["📬 Inbox cleanup:".to_string()];
            let mut sorted: Vec<_> = totals.iter().collect();
            sorted.sort_by_key(|(name, _)| name.as_str());
            for (name, rs) in &sorted {
                if rs.count > 0 {
                    let verb = match rs.action.as_str() {
                        "trash" => "trashed",
                        "label" => "labelled",
                        _ => "archived",
                    };
                    lines.push(format!("  • {}: {} {}", name, verb, rs.count));
                }
            }
            lines.join("\n")
        } else {
            "📬 Inbox check: all clean".to_string()
        };

        if let Err(e) = send_telegram_push(&store.sqlite, &message).await {
            warn!("JunkMailArchive: telegram push failed: {e}");
        }

        Ok(schedule_next_archive(&store.sqlite))
    }
}
