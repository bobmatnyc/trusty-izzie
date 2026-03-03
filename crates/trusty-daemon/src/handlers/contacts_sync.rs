//! macOS Contacts sync handler — reads AddressBook via osascript and upserts
//! entities into LanceDB.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::{info, warn};
use trusty_core::error::TrustyError;
use trusty_models::entity::{Entity, EntityType};
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;
use uuid::Uuid;

use super::{DispatchResult, EventHandler};

pub struct ContactsSyncHandler;

#[async_trait]
impl EventHandler for ContactsSyncHandler {
    fn event_type(&self) -> EventType {
        EventType::ContactsSync
    }

    async fn handle(
        &self,
        event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let force = match &event.payload {
            EventPayload::ContactsSync { force } => *force,
            _ => false,
        };

        info!(force, "Starting macOS Contacts sync");

        let raw = tokio::task::spawn_blocking(ContactsSyncHandler::fetch_contacts_via_osascript)
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;

        let contacts = Self::parse_contacts(&raw);
        info!(count = contacts.len(), "Parsed contacts from AddressBook");

        if contacts.is_empty() {
            warn!("No contacts returned from osascript — permissions may not be granted");
            return Ok(DispatchResult::Done);
        }

        let user_id = store.lance.user_id.clone();
        let mut synced = 0usize;
        let now = chrono::Utc::now();

        for contact in &contacts {
            let normalized = contact.full_name.to_lowercase();
            let fp = format!("contacts:{}", normalized);

            let entity_id = Uuid::new_v4().to_string();
            let sqlite = store.sqlite.clone();
            let fp_clone = fp.clone();
            let entity_id_clone = entity_id.clone();
            let normalized_clone = normalized.clone();

            let is_new = tokio::task::spawn_blocking(move || {
                sqlite.upsert_fingerprint(&fp_clone, &entity_id_clone, "Person", &normalized_clone)
            })
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;

            if is_new || force {
                let mut ctx_parts = Vec::new();
                if let Some(ref e) = contact.email {
                    ctx_parts.push(format!("email:{e}"));
                }
                if let Some(ref p) = contact.phone {
                    ctx_parts.push(format!("phone:{p}"));
                }
                if let Some(ref c) = contact.company {
                    ctx_parts.push(format!("company:{c}"));
                }
                let context_str = ctx_parts.join(", ");

                let entity_uuid = Uuid::parse_str(&entity_id).unwrap_or_else(|_| Uuid::new_v4());

                let entity = Entity {
                    id: entity_uuid,
                    user_id: user_id.clone(),
                    entity_type: EntityType::Person,
                    value: contact.full_name.clone(),
                    normalized: normalized.clone(),
                    confidence: 0.95,
                    source: "addressbook".to_string(),
                    source_id: Some(fp.clone()),
                    context: if context_str.is_empty() {
                        None
                    } else {
                        Some(context_str)
                    },
                    aliases: vec![],
                    occurrence_count: 1,
                    first_seen: now,
                    last_seen: now,
                    created_at: now,
                };

                if let Err(e) = store.lance.upsert_entity(&entity, vec![0.0f32; 384]).await {
                    warn!(name = %contact.full_name, error = %e, "Failed to upsert contact entity");
                } else {
                    synced += 1;
                }
            }
        }

        info!(
            total = contacts.len(),
            new_or_updated = synced,
            "Contacts sync complete"
        );
        Ok(DispatchResult::Done)
    }
}

struct ContactRecord {
    full_name: String,
    email: Option<String>,
    phone: Option<String>,
    company: Option<String>,
}

impl ContactsSyncHandler {
    /// Run osascript to export all contacts as tab-separated lines.
    /// Each line: FirstName\tLastName\tEmail\tPhone\tCompany
    fn fetch_contacts_via_osascript() -> anyhow::Result<String> {
        let script = r#"
set output to ""
tell application "Contacts"
    repeat with p in every person
        set fn to ""
        set ln to ""
        set em to ""
        set ph to ""
        set co to ""
        try
            set fn to first name of p
        end try
        try
            set ln to last name of p
        end try
        try
            if (count of emails of p) > 0 then
                set em to value of first item of emails of p
            end if
        end try
        try
            if (count of phones of p) > 0 then
                set ph to value of first item of phones of p
            end if
        end try
        try
            set co to organization of p
        end try
        set output to output & fn & "\t" & ln & "\t" & em & "\t" & ph & "\t" & co & "\n"
    end repeat
end tell
return output
"#;
        let out = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("osascript failed: {err}");
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Parse tab-separated contact lines into structured records.
    fn parse_contacts(raw: &str) -> Vec<ContactRecord> {
        raw.lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(5, '\t').collect();
                if parts.len() < 2 {
                    return None;
                }
                let first = parts[0].trim();
                let last = parts.get(1).map(|s| s.trim()).unwrap_or("");
                let full_name = format!("{} {}", first, last).trim().to_string();
                if full_name.is_empty() {
                    return None;
                }
                Some(ContactRecord {
                    full_name,
                    email: parts
                        .get(2)
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty()),
                    phone: parts
                        .get(3)
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty()),
                    company: parts
                        .get(4)
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty()),
                })
            })
            .collect()
    }
}
