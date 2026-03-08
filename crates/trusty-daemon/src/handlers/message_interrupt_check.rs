//! Polls iMessage and WhatsApp every 5 minutes for messages needing attention.

use anyhow::Context as _;
use async_trait::async_trait;
use chrono::Utc;
use rusqlite::{Connection, OpenFlags};
use std::sync::Arc;
use tracing::{info, warn};

/// (source, contact, text) triple returned from each DB query.
type MsgRow = (i64, Vec<(String, String, String)>);

use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use crate::handlers::{DispatchResult, EventHandler};
use crate::telegram_push::send_telegram_push;

pub struct MessageInterruptCheckHandler;

#[async_trait]
impl EventHandler for MessageInterruptCheckHandler {
    fn event_type(&self) -> EventType {
        EventType::MessageInterruptCheck
    }

    async fn handle(
        &self,
        _event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let result = run_check(store).await;
        if let Err(e) = &result {
            warn!("MessageInterruptCheck failed: {e}");
        }
        // Always reschedule in 5 minutes regardless of errors.
        let next = Utc::now().timestamp() + 300;
        Ok(DispatchResult::Chain(vec![(
            EventType::MessageInterruptCheck,
            EventPayload::MessageInterruptCheck {},
            next,
        )]))
    }
}

async fn run_check(store: &Arc<Store>) -> Result<(), anyhow::Error> {
    let home = std::env::var("HOME").unwrap_or_default();
    let api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();

    // --- Read cursors ---
    let imsg_cursor: i64 = store
        .sqlite
        .get_config("message_interrupt_last_imsg_rowid")
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(189791);

    let wa_cursor: i64 = store
        .sqlite
        .get_config("message_interrupt_last_wa_pk")
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(25650);

    // --- Read new iMessages (received only, is_from_me = 0) ---
    let imsg_db = format!("{home}/Library/Messages/chat.db");
    let (new_imsg_rowid, imessages) = tokio::task::spawn_blocking({
        let imsg_db = imsg_db.clone();
        move || -> Result<MsgRow, anyhow::Error> {
            let conn = Connection::open_with_flags(
                &imsg_db,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .context("open iMessage DB")?;
            let mut stmt = conn.prepare(
                "SELECT m.rowid, h.id, m.text \
                 FROM message m JOIN handle h ON m.handle_id = h.rowid \
                 WHERE m.rowid > ?1 AND m.is_from_me = 0 AND m.text IS NOT NULL \
                 ORDER BY m.rowid ASC LIMIT 50",
            )?;
            let rows: Vec<(i64, String, String)> = stmt
                .query_map([imsg_cursor], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();

            let max_rowid = rows.last().map(|(id, _, _)| *id).unwrap_or(imsg_cursor);
            let messages = rows
                .into_iter()
                .map(|(_, contact, text)| ("iMessage".to_string(), contact, text))
                .collect();
            Ok((max_rowid, messages))
        }
    })
    .await??;

    // --- Read new WhatsApp messages (received only, ZISFROMME = 0) ---
    let wa_db = format!(
        "{home}/Library/Group Containers/group.net.whatsapp.WhatsApp.shared/ChatStorage.sqlite"
    );
    let (new_wa_pk, whatsapp_messages) = tokio::task::spawn_blocking({
        let wa_db = wa_db.clone();
        move || -> Result<MsgRow, anyhow::Error> {
            let conn = Connection::open_with_flags(
                &wa_db,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .context("open WhatsApp DB")?;
            let mut stmt = conn.prepare(
                "SELECT m.Z_PK, COALESCE(s.ZPARTNERNAME, s.ZCONTACTJID, 'Unknown'), m.ZTEXT \
                 FROM ZWAMESSAGE m JOIN ZWACHATSESSION s ON m.ZCHATSESSION = s.Z_PK \
                 WHERE m.Z_PK > ?1 AND m.ZISFROMME = 0 AND m.ZTEXT IS NOT NULL \
                 ORDER BY m.Z_PK ASC LIMIT 50",
            )?;
            let rows: Vec<(i64, String, String)> = stmt
                .query_map([wa_cursor], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();

            let max_pk = rows.last().map(|(id, _, _)| *id).unwrap_or(wa_cursor);
            let messages = rows
                .into_iter()
                .map(|(_, contact, text)| ("WhatsApp".to_string(), contact, text))
                .collect();
            Ok((max_pk, messages))
        }
    })
    .await??;

    // --- Update cursors immediately ---
    store.sqlite.set_config(
        "message_interrupt_last_imsg_rowid",
        &new_imsg_rowid.to_string(),
    )?;
    store
        .sqlite
        .set_config("message_interrupt_last_wa_pk", &new_wa_pk.to_string())?;

    let all_messages: Vec<(String, String, String)> =
        imessages.into_iter().chain(whatsapp_messages).collect();

    if all_messages.is_empty() {
        info!("MessageInterruptCheck: no new messages");
        return Ok(());
    }

    info!(
        "MessageInterruptCheck: {} new messages, classifying with Haiku",
        all_messages.len()
    );

    // --- Classify with Haiku ---
    if api_key.is_empty() {
        warn!("OPENROUTER_API_KEY not set, skipping classification");
        return Ok(());
    }

    let messages_text: String = all_messages
        .iter()
        .enumerate()
        .map(|(i, (source, contact, text))| {
            format!("{}. [{}] From {}: {}", i + 1, source, contact, text)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let system_prompt = "You are an assistant that helps filter messages for a busy professional. \
        For each message, decide if it NEEDS_ATTENTION (requires a reply, is urgent, contains a question, \
        a request, or time-sensitive info) or SKIP (spam, notifications, OTP codes, marketing, \
        casual FYI with no action needed). \
        Respond with one line per message: the number, a pipe, then NEEDS_ATTENTION or SKIP, a pipe, then a 5-word-max reason. \
        Example: 1|NEEDS_ATTENTION|Meeting request from Alice";

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let resp = client
        .post("https://openrouter.ai/api/v1/chat/completions")
        .bearer_auth(&api_key)
        .json(&serde_json::json!({
            "model": "anthropic/claude-haiku-4-6",
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": messages_text}
            ],
            "max_tokens": 300,
            "temperature": 0.0
        }))
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    let classification = resp["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    // --- Parse and notify ---
    let needs_attention: Vec<(&(String, String, String), &str)> = classification
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, '|').collect();
            if parts.len() >= 2 && parts[1].trim() == "NEEDS_ATTENTION" {
                let idx: usize = parts[0].trim().parse::<usize>().ok()?.saturating_sub(1);
                let reason = if parts.len() >= 3 {
                    parts[2].trim()
                } else {
                    ""
                };
                all_messages.get(idx).map(|msg| (msg, reason))
            } else {
                None
            }
        })
        .collect();

    if needs_attention.is_empty() {
        info!(
            "MessageInterruptCheck: {} messages, none need attention",
            all_messages.len()
        );
        return Ok(());
    }

    // Build notification.
    let mut notification = format!(
        "<b>{} message{} need your attention:</b>\n\n",
        needs_attention.len(),
        if needs_attention.len() == 1 { "" } else { "s" }
    );
    for (msg, reason) in &needs_attention {
        let preview = if msg.2.len() > 100 {
            format!("{}...", &msg.2[..100])
        } else {
            msg.2.clone()
        };
        notification.push_str(&format!(
            "- <b>{}</b> via {}\n  {}\n  <i>{}</i>\n\n",
            msg.1, msg.0, preview, reason
        ));
    }

    send_telegram_push(&store.sqlite, &notification).await?;
    info!(
        "MessageInterruptCheck: notified about {} messages",
        needs_attention.len()
    );
    Ok(())
}
