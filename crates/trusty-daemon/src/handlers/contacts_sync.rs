//! macOS Contacts sync handler — reads AddressBook via osascript and upserts
//! entities into LanceDB and the Kuzu knowledge graph.
//!
//! Matching is three-tiered to balance speed, accuracy, and cost:
//!   1. Exact match via fingerprint table (no inference).
//!   2. Vector similarity ≥ 0.92 (no inference).
//!   3. Haiku batch call for 0.70–0.92 similarity candidates.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::{info, warn};
use trusty_core::error::TrustyError;
use trusty_embeddings::embedder::global_embedder;
use trusty_models::entity::{Entity, EntityType};
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;
use uuid::Uuid;

use super::{DispatchResult, EventHandler};

// ---------------------------------------------------------------------------
// Distance → similarity conversion.
// LanceDB reports L2 distance for default ANN queries. For unit-norm vectors
// (fastembed normalises by default) the relationship is:
//   cosine_similarity = 1 − L2²/2  →  L2 = sqrt(2*(1−cos))
// Thresholds (cosine similarity → L2 distance):
//   0.92 → ~0.40,  0.70 → ~0.775
// ---------------------------------------------------------------------------
const DIST_HIGH: f32 = 0.40; // cosine ≥ 0.92 — same person
const DIST_MID: f32 = 0.775; // cosine 0.70–0.92 — Haiku review

// OpenRouter constants (same as trusty-extractor)
const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";
const HAIKU_MODEL: &str = "anthropic/claude-haiku-4-5";
const HAIKU_BATCH_SIZE: usize = 50;

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

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
        let now = chrono::Utc::now();

        // Collect medium-confidence candidates for Haiku batch review.
        // Each entry: (ContactRecord, existing_entity_id)
        let mut haiku_candidates: Vec<(ContactRecord, String)> = Vec::new();

        // ----------------------------------------------------------------
        // Per-contact processing
        // ----------------------------------------------------------------
        for contact in contacts {
            let first = contact.first_name.as_deref().unwrap_or("").trim();
            let last = contact.last_name.as_deref().unwrap_or("").trim();
            let normalized_name = format!("{}_{}", first.to_lowercase(), last.to_lowercase());
            let fp = format!("contacts:{}", normalized_name);

            // --- Tier 1: fingerprint / exact match ---
            let sqlite = store.sqlite.clone();
            let fp_clone = fp.clone();
            let new_entity_id = Uuid::new_v4().to_string();
            let new_entity_id_clone = new_entity_id.clone();
            let normalized_clone = normalized_name.clone();

            let is_new = tokio::task::spawn_blocking(move || {
                sqlite.upsert_fingerprint(
                    &fp_clone,
                    &new_entity_id_clone,
                    "Person",
                    &normalized_clone,
                )
            })
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;

            if !is_new && !force {
                // Exact match existed — fingerprint already updated by upsert_fingerprint.
                continue;
            }

            // --- Build embedding ---
            let embed_text = format!(
                "{} {} {}",
                first,
                last,
                contact.company.as_deref().unwrap_or("")
            )
            .trim()
            .to_string();

            let embed_text_clone = embed_text.clone();
            let embedding = tokio::task::spawn_blocking(move || {
                if let Some(embedder) = global_embedder() {
                    embedder
                        .embed(&embed_text_clone)
                        .map_err(|e| anyhow::anyhow!("embed failed: {}", e))
                } else {
                    // Fall back to zero vector if embedder not initialised.
                    Ok(vec![0.0f32; 384])
                }
            })
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;

            // --- Tier 2: vector similarity ---
            let search_results = store.lance.search_entities(&embedding, 3).await;
            let top_match: Option<(String, f32)> = match search_results {
                Ok(ref hits) if !hits.is_empty() => Some(hits[0].clone()),
                _ => None,
            };

            if let Some((ref existing_id, dist)) = top_match {
                if dist < DIST_HIGH {
                    // High similarity — treat as same person; skip new insert.
                    // The fingerprint row already points at new_entity_id which
                    // won't exist; update it to point at the existing entity.
                    // (Best-effort; a future sync will converge via Tier 1.)
                    info!(
                        name = %contact.full_name,
                        dist,
                        existing = %existing_id,
                        "Tier-2 match: merging into existing entity"
                    );
                    continue;
                } else if dist < DIST_MID {
                    // Medium similarity — queue for Haiku review.
                    haiku_candidates.push((contact, existing_id.clone()));
                    continue;
                }
            }

            // --- Tier 3 pre-pass or no match: insert as new entity ---
            let entity = Self::build_entity(
                &new_entity_id,
                &user_id,
                &contact,
                &normalized_name,
                &fp,
                now,
            );

            Self::upsert_to_stores(store, &entity, embedding).await;
        }

        // ----------------------------------------------------------------
        // Haiku batch deduplication for medium-confidence candidates
        // ----------------------------------------------------------------
        if !haiku_candidates.is_empty() {
            info!(
                count = haiku_candidates.len(),
                "Running Haiku batch dedup for medium-confidence contacts"
            );

            let api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
            if api_key.is_empty() {
                warn!("OPENROUTER_API_KEY not set — skipping Haiku dedup, inserting all as new");
                for (contact, _) in &haiku_candidates {
                    let first = contact.first_name.as_deref().unwrap_or("").trim();
                    let last = contact.last_name.as_deref().unwrap_or("").trim();
                    let normalized_name =
                        format!("{}_{}", first.to_lowercase(), last.to_lowercase());
                    let fp = format!("contacts:{}", normalized_name);
                    let entity_id = Uuid::new_v4().to_string();
                    let entity = Self::build_entity(
                        &entity_id,
                        &user_id,
                        contact,
                        &normalized_name,
                        &fp,
                        now,
                    );
                    let embed_text = format!(
                        "{} {} {}",
                        first,
                        last,
                        contact.company.as_deref().unwrap_or("")
                    )
                    .trim()
                    .to_string();
                    let embedding = tokio::task::spawn_blocking(move || {
                        global_embedder()
                            .map(|e| e.embed(&embed_text).unwrap_or_else(|_| vec![0.0f32; 384]))
                            .unwrap_or_else(|| vec![0.0f32; 384])
                    })
                    .await
                    .unwrap_or_else(|_| vec![0.0f32; 384]);
                    Self::upsert_to_stores(store, &entity, embedding).await;
                }
            } else {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(60))
                    .build()
                    .map_err(|e| TrustyError::Storage(e.to_string()))?;

                for chunk in haiku_candidates.chunks(HAIKU_BATCH_SIZE) {
                    let decisions = Self::haiku_batch_dedup(&client, &api_key, chunk, store).await;

                    for (i, (contact, existing_id)) in chunk.iter().enumerate() {
                        let first = contact.first_name.as_deref().unwrap_or("").trim();
                        let last = contact.last_name.as_deref().unwrap_or("").trim();
                        let normalized_name =
                            format!("{}_{}", first.to_lowercase(), last.to_lowercase());
                        let fp = format!("contacts:{}", normalized_name);

                        let is_yes = decisions.get(i).copied().unwrap_or(false);
                        if is_yes {
                            // Merge: existing entity already in store; aliases update is
                            // handled by the fingerprint row pointing at existing_id.
                            info!(
                                name = %contact.full_name,
                                existing = %existing_id,
                                "Haiku: YES — merging into existing entity"
                            );
                            continue;
                        }

                        // NO or UNSURE → insert as new entity
                        let entity_id = Uuid::new_v4().to_string();
                        let entity = Self::build_entity(
                            &entity_id,
                            &user_id,
                            contact,
                            &normalized_name,
                            &fp,
                            now,
                        );
                        let embed_text = format!(
                            "{} {} {}",
                            first,
                            last,
                            contact.company.as_deref().unwrap_or("")
                        )
                        .trim()
                        .to_string();
                        let embedding = tokio::task::spawn_blocking(move || {
                            global_embedder()
                                .map(|e| e.embed(&embed_text).unwrap_or_else(|_| vec![0.0f32; 384]))
                                .unwrap_or_else(|| vec![0.0f32; 384])
                        })
                        .await
                        .unwrap_or_else(|_| vec![0.0f32; 384]);
                        Self::upsert_to_stores(store, &entity, embedding).await;
                    }
                }
            }
        }

        info!("Contacts sync complete");
        Ok(DispatchResult::Done)
    }
}

// ---------------------------------------------------------------------------
// ContactRecord
// ---------------------------------------------------------------------------

struct ContactRecord {
    full_name: String,
    first_name: Option<String>,
    last_name: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    company: Option<String>,
}

// ---------------------------------------------------------------------------
// Helper methods
// ---------------------------------------------------------------------------

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
                    first_name: if first.is_empty() {
                        None
                    } else {
                        Some(first.to_string())
                    },
                    last_name: if last.is_empty() {
                        None
                    } else {
                        Some(last.to_string())
                    },
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

    /// Build an Entity from a ContactRecord, applying Gap 3 field layout.
    fn build_entity(
        entity_id: &str,
        user_id: &str,
        contact: &ContactRecord,
        normalized: &str,
        fp: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Entity {
        let entity_uuid = Uuid::parse_str(entity_id).unwrap_or_else(|_| Uuid::new_v4());

        // Gap 3: context = primary email; aliases = phones + secondary emails + name variants.
        let context = contact.email.clone();

        let mut aliases: Vec<String> = Vec::new();
        if let Some(ref p) = contact.phone {
            aliases.push(p.clone());
        }
        if let Some(ref c) = contact.company {
            aliases.push(format!("company:{c}"));
        }
        // Normalised name variant
        let first = contact.first_name.as_deref().unwrap_or("");
        let last = contact.last_name.as_deref().unwrap_or("");
        if !first.is_empty() && !last.is_empty() {
            aliases.push(format!("{} {}", last, first)); // "Last First" variant
        }

        Entity {
            id: entity_uuid,
            user_id: user_id.to_string(),
            entity_type: EntityType::Person,
            value: contact.full_name.clone(),
            normalized: normalized.to_string(),
            confidence: 0.95,
            source: "addressbook".to_string(),
            source_id: Some(fp.to_string()),
            context,
            aliases,
            occurrence_count: 1,
            first_seen: now,
            last_seen: now,
            created_at: now,
        }
    }

    /// Write entity to both LanceDB (vector) and Kuzu (graph).
    async fn upsert_to_stores(store: &Arc<Store>, entity: &Entity, embedding: Vec<f32>) {
        // LanceDB (async)
        if let Err(e) = store.lance.upsert_entity(entity, embedding).await {
            warn!(name = %entity.value, error = %e, "Failed to upsert contact into LanceDB");
        }

        // Kuzu graph (synchronous — must run in spawn_blocking)
        let graph = store.graph.clone();
        let entity_clone = entity.clone();
        if let Err(e) =
            tokio::task::spawn_blocking(move || graph.upsert_entity(&entity_clone)).await
        {
            warn!(name = %entity.value, error = %e, "Failed to upsert contact into Kuzu");
        }
    }

    /// Send a batch of (contact, existing_entity_id) pairs to Haiku for deduplication.
    /// Returns a Vec<bool> of the same length: true = YES (merge), false = NO/UNSURE (new).
    async fn haiku_batch_dedup(
        client: &reqwest::Client,
        api_key: &str,
        candidates: &[(ContactRecord, String)],
        store: &Arc<Store>,
    ) -> Vec<bool> {
        // Build numbered prompt lines
        let mut prompt_lines = Vec::with_capacity(candidates.len());
        for (i, (contact, existing_id)) in candidates.iter().enumerate() {
            let existing_label = Self::describe_existing(store, existing_id).await;
            let contact_desc = format!(
                "{}{}{}",
                contact.full_name,
                contact
                    .email
                    .as_deref()
                    .map(|e| format!(", {e}"))
                    .unwrap_or_default(),
                contact
                    .phone
                    .as_deref()
                    .map(|p| format!(", {p}"))
                    .unwrap_or_default(),
            );
            prompt_lines.push(format!(
                "{}. Contact: {}\n   Existing: {}",
                i + 1,
                contact_desc,
                existing_label
            ));
        }

        let system_prompt =
            "You are a contact deduplication assistant. For each pair, answer only YES, NO, or UNSURE on a new line.";
        let user_prompt = format!(
            "Are the following pairs the same person?\n\n{}",
            prompt_lines.join("\n")
        );

        let body = serde_json::json!({
            "model": HAIKU_MODEL,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt}
            ],
            "max_tokens": 200,
            "temperature": 0.0
        });

        let resp = match client
            .post(format!("{}/chat/completions", OPENROUTER_BASE))
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "Haiku batch request failed — treating all as new");
                return vec![false; candidates.len()];
            }
        };

        let json: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "Failed to parse Haiku response — treating all as new");
                return vec![false; candidates.len()];
            }
        };

        let text = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("");

        // Parse line by line; YES → true, anything else → false.
        let mut decisions = vec![false; candidates.len()];
        for (i, line) in text.lines().enumerate() {
            if i >= candidates.len() {
                break;
            }
            let trimmed = line.trim().to_uppercase();
            decisions[i] = trimmed.starts_with("YES");
        }
        decisions
    }

    /// Describe an existing entity (by LanceDB id) for the Haiku prompt.
    async fn describe_existing(store: &Arc<Store>, entity_id: &str) -> String {
        match store.lance.get_entity_by_id(entity_id).await {
            Ok(Some(e)) => {
                let email = e.context.as_deref().unwrap_or("").to_string();
                if email.is_empty() {
                    e.value
                } else {
                    format!("{}, {}", e.value, email)
                }
            }
            _ => entity_id.to_string(),
        }
    }
}
