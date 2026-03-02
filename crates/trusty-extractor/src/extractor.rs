//! The `EntityExtractor` struct and its extraction logic.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use uuid::Uuid;

use trusty_models::email::EmailMessage;
use trusty_models::entity::{
    Entity, EntityType, Relationship, RelationshipStatus, RelationshipType,
};

use crate::prompt::{EXTRACTION_PROMPT, TEXT_EXTRACTION_PROMPT};
use crate::types::{ExtractionResult, UserContext};

/// Configuration for the LLM extraction client.
#[derive(Debug, Clone)]
pub struct ExtractorConfig {
    /// OpenRouter API base URL.
    pub base_url: String,
    /// API key for OpenRouter.
    pub api_key: String,
    /// Model identifier (e.g. `"mistralai/mistral-small-3.1-24b-instruct"`).
    pub model: String,
    /// Maximum tokens in the extraction response.
    pub max_tokens: u32,
    /// Confidence threshold — entities below this are discarded.
    pub confidence_threshold: f32,
    /// Maximum relationships to keep per email.
    pub max_relationships: usize,
}

/// Calls an LLM via OpenRouter to extract entities and relationships from email.
pub struct EntityExtractor {
    config: ExtractorConfig,
    http: reqwest::Client,
}

/// Minimal OpenRouter chat completion request body.
#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    max_tokens: u32,
    temperature: f32,
    response_format: ResponseFormat,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ResponseFormat {
    r#type: &'static str,
}

/// Partial OpenRouter chat completion response.
#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    message: AssistantMessage,
}

#[derive(Deserialize)]
struct AssistantMessage {
    content: String,
}

#[derive(Deserialize)]
struct Usage {
    total_tokens: u32,
}

/// Raw entity shape as returned by the LLM.
#[derive(Deserialize)]
struct RawEntity {
    entity_type: String,
    value: String,
    normalized: String,
    confidence: f32,
    source: String,
    context: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
}

/// Raw relationship shape as returned by the LLM.
#[derive(Deserialize)]
struct RawRelationship {
    from_entity_value: String,
    from_entity_type: String,
    to_entity_value: String,
    to_entity_type: String,
    relationship_type: String,
    confidence: f32,
    evidence: Option<String>,
}

impl EntityExtractor {
    /// Construct a new extractor with the given configuration.
    pub fn new(config: ExtractorConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("failed to build reqwest client");
        Self { config, http }
    }

    /// Extract entities and relationships from a single email.
    ///
    /// Returns an `ExtractionResult` even on partial failure so the caller
    /// can decide whether to retry or skip.
    pub async fn extract_from_email(
        &self,
        email: &EmailMessage,
        user_context: &UserContext,
    ) -> Result<ExtractionResult> {
        // Build the prompt by substituting placeholders
        let user_ctx_json = serde_json::to_string_pretty(user_context)?;
        let email_text = format_email_for_extraction(email);

        let prompt = EXTRACTION_PROMPT
            .replace("{{USER_CONTEXT}}", &user_ctx_json)
            .replace("{{EMAIL_CONTENT}}", &email_text);

        debug!(email_id = %email.id, model = %self.config.model, "extracting entities");

        let request_body = ChatRequest {
            model: &self.config.model,
            messages: vec![
                Message {
                    role: "system",
                    content: "You are an entity extraction assistant. Respond only with JSON.",
                },
                Message {
                    role: "user",
                    content: &prompt,
                },
            ],
            max_tokens: self.config.max_tokens,
            temperature: 0.0,
            response_format: ResponseFormat {
                r#type: "json_object",
            },
        };

        let response = self
            .http
            .post(format!("{}/chat/completions", self.config.base_url))
            .bearer_auth(&self.config.api_key)
            .json(&request_body)
            .send()
            .await?
            .error_for_status()?
            .json::<ChatResponse>()
            .await?;

        let content = response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default();

        let tokens_used = response.usage.map(|u| u.total_tokens).unwrap_or(0);

        self.parse_extraction_response(&content, email, user_context, tokens_used)
    }

    /// Extract entities and relationships from free-form text (chat, documents).
    ///
    /// Unlike `extract_from_email`, persons are allowed from any part of the text
    /// where they are explicitly named (not restricted to headers).
    pub async fn extract_from_text(
        &self,
        text: &str,
        source_context: &str,
        user_context: &UserContext,
    ) -> Result<ExtractionResult> {
        let user_ctx_json = serde_json::to_string_pretty(user_context)?;

        let prompt = TEXT_EXTRACTION_PROMPT
            .replace("{{USER_CONTEXT}}", &user_ctx_json)
            .replace("{{TEXT_CONTENT}}", text);

        debug!(source = %source_context, model = %self.config.model, "extracting entities from text");

        let request_body = ChatRequest {
            model: &self.config.model,
            messages: vec![
                Message {
                    role: "system",
                    content: "You are an entity extraction assistant. Respond only with JSON.",
                },
                Message {
                    role: "user",
                    content: &prompt,
                },
            ],
            max_tokens: self.config.max_tokens,
            temperature: 0.0,
            response_format: ResponseFormat {
                r#type: "json_object",
            },
        };

        let response = self
            .http
            .post(format!("{}/chat/completions", self.config.base_url))
            .bearer_auth(&self.config.api_key)
            .json(&request_body)
            .send()
            .await?
            .error_for_status()?
            .json::<ChatResponse>()
            .await?;

        let content = response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default();

        let tokens_used = response.usage.map(|u| u.total_tokens).unwrap_or(0);

        let mut result = self.parse_extraction_response_text(
            &content,
            source_context,
            user_context,
            tokens_used,
        )?;

        // Tag each entity with the source context identifier.
        for entity in &mut result.entities {
            entity.source_id = Some(source_context.to_string());
        }

        Ok(result)
    }

    /// Parse the raw JSON string returned by the LLM into an `ExtractionResult`.
    fn parse_extraction_response(
        &self,
        json_str: &str,
        email: &EmailMessage,
        user_context: &UserContext,
        tokens_used: u32,
    ) -> Result<ExtractionResult> {
        // Parse into loosely-typed Value first to handle partial outputs.
        let raw: serde_json::Value = serde_json::from_str(json_str).map_err(|e| {
            warn!(email_id = %email.id, error = %e, "failed to parse extraction JSON");
            e
        })?;

        let skipped_noise = raw
            .get("skipped_noise")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if skipped_noise {
            return Ok(ExtractionResult {
                entities: vec![],
                relationships: vec![],
                overall_confidence: 1.0,
                tokens_used,
                skipped_noise: true,
            });
        }

        let overall_confidence = raw
            .get("overall_confidence")
            .and_then(|v| v.as_f64())
            .map(|f| f as f32)
            .unwrap_or(0.0);

        let now = Utc::now();

        // --- Entities ---
        let user_normalized = normalize_for_match(&user_context.email);
        let user_name_normalized = normalize_for_match(&user_context.display_name);

        let entities: Vec<Entity> = raw
            .get("entities")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let raw_entity: RawEntity = match serde_json::from_value(item.clone()) {
                            Ok(e) => e,
                            Err(err) => {
                                warn!(email_id = %email.id, error = %err, "skipping unparseable entity");
                                return None;
                            }
                        };

                        // Filter by confidence threshold.
                        if raw_entity.confidence < self.config.confidence_threshold {
                            return None;
                        }

                        // Persons must not come from the email body.
                        if raw_entity.entity_type.to_lowercase() == "person"
                            && raw_entity.source == "body"
                        {
                            return None;
                        }

                        // Skip the user themselves.
                        let norm = &raw_entity.normalized;
                        if normalize_for_match(norm) == user_normalized
                            || normalize_for_match(norm) == user_name_normalized
                        {
                            return None;
                        }

                        let entity_type = match parse_entity_type(&raw_entity.entity_type) {
                            Some(t) => t,
                            None => {
                                warn!(
                                    email_id = %email.id,
                                    entity_type = %raw_entity.entity_type,
                                    "skipping entity with unknown type"
                                );
                                return None;
                            }
                        };

                        Some(Entity {
                            id: Uuid::new_v4(),
                            user_id: user_context.user_id.clone(),
                            entity_type,
                            value: raw_entity.value,
                            normalized: raw_entity.normalized,
                            confidence: raw_entity.confidence,
                            source: raw_entity.source,
                            source_id: Some(email.id.clone()),
                            context: raw_entity.context,
                            aliases: raw_entity.aliases,
                            occurrence_count: 1,
                            first_seen: now,
                            last_seen: now,
                            created_at: now,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        // --- Relationships ---
        let mut raw_rels: Vec<RawRelationship> = raw
            .get("relationships")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let rel: RawRelationship = match serde_json::from_value(item.clone()) {
                            Ok(r) => r,
                            Err(err) => {
                                warn!(email_id = %email.id, error = %err, "skipping unparseable relationship");
                                return None;
                            }
                        };
                        if rel.confidence < self.config.confidence_threshold {
                            return None;
                        }
                        Some(rel)
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Sort descending by confidence, then cap at max_relationships.
        raw_rels.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        raw_rels.truncate(self.config.max_relationships);

        let relationships: Vec<Relationship> = raw_rels
            .into_iter()
            .filter_map(|rel| {
                // Skip if either endpoint is the user.
                let from_norm = normalize_for_match(&rel.from_entity_value);
                let to_norm = normalize_for_match(&rel.to_entity_value);
                if from_norm == user_normalized
                    || from_norm == user_name_normalized
                    || to_norm == user_normalized
                    || to_norm == user_name_normalized
                {
                    return None;
                }

                let from_type = match parse_entity_type(&rel.from_entity_type) {
                    Some(t) => t,
                    None => {
                        warn!(
                            email_id = %email.id,
                            from_type = %rel.from_entity_type,
                            "skipping relationship with unknown from_entity_type"
                        );
                        return None;
                    }
                };
                let to_type = match parse_entity_type(&rel.to_entity_type) {
                    Some(t) => t,
                    None => {
                        warn!(
                            email_id = %email.id,
                            to_type = %rel.to_entity_type,
                            "skipping relationship with unknown to_entity_type"
                        );
                        return None;
                    }
                };
                let relationship_type = match parse_relationship_type(&rel.relationship_type) {
                    Some(t) => t,
                    None => {
                        warn!(
                            email_id = %email.id,
                            rel_type = %rel.relationship_type,
                            "skipping relationship with unknown relationship_type"
                        );
                        return None;
                    }
                };

                Some(Relationship {
                    id: Uuid::new_v4(),
                    user_id: user_context.user_id.clone(),
                    from_entity_type: from_type,
                    from_entity_value: rel.from_entity_value,
                    to_entity_type: to_type,
                    to_entity_value: rel.to_entity_value,
                    relationship_type,
                    confidence: rel.confidence,
                    evidence: rel.evidence,
                    source_id: Some(email.id.clone()),
                    status: RelationshipStatus::Unknown,
                    first_seen: now,
                    last_seen: now,
                })
            })
            .collect();

        Ok(ExtractionResult {
            entities,
            relationships,
            overall_confidence,
            tokens_used,
            skipped_noise: false,
        })
    }

    /// Like `parse_extraction_response` but for free-form text:
    /// persons are allowed from any source (no body filter).
    fn parse_extraction_response_text(
        &self,
        json_str: &str,
        source_context: &str,
        user_context: &UserContext,
        tokens_used: u32,
    ) -> Result<ExtractionResult> {
        let raw: serde_json::Value = serde_json::from_str(json_str).map_err(|e| {
            warn!(source = %source_context, error = %e, "failed to parse text extraction JSON");
            e
        })?;

        let skipped_noise = raw
            .get("skipped_noise")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if skipped_noise {
            return Ok(ExtractionResult {
                entities: vec![],
                relationships: vec![],
                overall_confidence: 1.0,
                tokens_used,
                skipped_noise: true,
            });
        }

        let overall_confidence = raw
            .get("overall_confidence")
            .and_then(|v| v.as_f64())
            .map(|f| f as f32)
            .unwrap_or(0.0);

        let now = Utc::now();
        let user_normalized = normalize_for_match(&user_context.email);
        let user_name_normalized = normalize_for_match(&user_context.display_name);

        let entities: Vec<Entity> = raw
            .get("entities")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let raw_entity: RawEntity = match serde_json::from_value(item.clone()) {
                            Ok(e) => e,
                            Err(err) => {
                                warn!(source = %source_context, error = %err, "skipping unparseable entity");
                                return None;
                            }
                        };

                        if raw_entity.confidence < self.config.confidence_threshold {
                            return None;
                        }

                        // Persons from text are always allowed — no source filter here.

                        let norm = &raw_entity.normalized;
                        if normalize_for_match(norm) == user_normalized
                            || normalize_for_match(norm) == user_name_normalized
                        {
                            return None;
                        }

                        let entity_type = match parse_entity_type(&raw_entity.entity_type) {
                            Some(t) => t,
                            None => {
                                warn!(
                                    source = %source_context,
                                    entity_type = %raw_entity.entity_type,
                                    "skipping entity with unknown type"
                                );
                                return None;
                            }
                        };

                        Some(Entity {
                            id: uuid::Uuid::new_v4(),
                            user_id: user_context.user_id.clone(),
                            entity_type,
                            value: raw_entity.value,
                            normalized: raw_entity.normalized,
                            confidence: raw_entity.confidence,
                            source: raw_entity.source,
                            source_id: None, // set by caller
                            context: raw_entity.context,
                            aliases: raw_entity.aliases,
                            occurrence_count: 1,
                            first_seen: now,
                            last_seen: now,
                            created_at: now,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut raw_rels: Vec<RawRelationship> = raw
            .get("relationships")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let rel: RawRelationship = match serde_json::from_value(item.clone()) {
                            Ok(r) => r,
                            Err(err) => {
                                warn!(source = %source_context, error = %err, "skipping unparseable relationship");
                                return None;
                            }
                        };
                        if rel.confidence < self.config.confidence_threshold {
                            return None;
                        }
                        Some(rel)
                    })
                    .collect()
            })
            .unwrap_or_default();

        raw_rels.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        raw_rels.truncate(self.config.max_relationships);

        let relationships: Vec<Relationship> = raw_rels
            .into_iter()
            .filter_map(|rel| {
                let from_norm = normalize_for_match(&rel.from_entity_value);
                let to_norm = normalize_for_match(&rel.to_entity_value);
                if from_norm == user_normalized
                    || from_norm == user_name_normalized
                    || to_norm == user_normalized
                    || to_norm == user_name_normalized
                {
                    return None;
                }

                let from_type = match parse_entity_type(&rel.from_entity_type) {
                    Some(t) => t,
                    None => return None,
                };
                let to_type = match parse_entity_type(&rel.to_entity_type) {
                    Some(t) => t,
                    None => return None,
                };
                let relationship_type = match parse_relationship_type(&rel.relationship_type) {
                    Some(t) => t,
                    None => return None,
                };

                Some(Relationship {
                    id: uuid::Uuid::new_v4(),
                    user_id: user_context.user_id.clone(),
                    from_entity_type: from_type,
                    from_entity_value: rel.from_entity_value,
                    to_entity_type: to_type,
                    to_entity_value: rel.to_entity_value,
                    relationship_type,
                    confidence: rel.confidence,
                    evidence: rel.evidence,
                    source_id: Some(source_context.to_string()),
                    status: RelationshipStatus::Unknown,
                    first_seen: now,
                    last_seen: now,
                })
            })
            .collect();

        Ok(ExtractionResult {
            entities,
            relationships,
            overall_confidence,
            tokens_used,
            skipped_noise: false,
        })
    }
}

/// Returns `true` if the email shows clear noise signals that make LLM
/// extraction wasteful. Checked before calling the LLM.
pub fn is_noise_email(email: &EmailMessage) -> bool {
    let from_lower = email.from.to_lowercase();
    let noise_senders = [
        "noreply",
        "no-reply",
        "donotreply",
        "notifications@",
        "alerts@",
        "mailer-daemon",
    ];
    if noise_senders.iter().any(|s| from_lower.contains(s)) {
        return true;
    }

    let subject_lower = email.subject.to_lowercase();
    let noise_subjects = ["[jira]", "[github]", "[slack]", "unsubscribe", "newsletter"];
    if noise_subjects.iter().any(|s| subject_lower.contains(s)) {
        return true;
    }

    let noise_labels = ["CATEGORY_PROMOTIONS", "CATEGORY_UPDATES"];
    if email
        .labels
        .iter()
        .any(|l| noise_labels.contains(&l.as_str()))
    {
        return true;
    }

    false
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Normalise a string for user-identity matching (lower, strip spaces).
fn normalize_for_match(s: &str) -> String {
    s.to_lowercase().replace([' ', '_', '-', '.'], "")
}

fn parse_entity_type(s: &str) -> Option<EntityType> {
    match s.to_lowercase().as_str() {
        "person" => Some(EntityType::Person),
        "company" => Some(EntityType::Company),
        "project" => Some(EntityType::Project),
        "tool" => Some(EntityType::Tool),
        "topic" => Some(EntityType::Topic),
        "location" => Some(EntityType::Location),
        "action_item" | "actionitem" => Some(EntityType::ActionItem),
        _ => None,
    }
}

fn parse_relationship_type(s: &str) -> Option<RelationshipType> {
    match s.to_uppercase().as_str() {
        "WORKS_WITH" => Some(RelationshipType::WorksWith),
        "WORKS_FOR" => Some(RelationshipType::WorksFor),
        "WORKS_ON" => Some(RelationshipType::WorksOn),
        "REPORTS_TO" => Some(RelationshipType::ReportsTo),
        "LEADS" => Some(RelationshipType::Leads),
        "EXPERT_IN" => Some(RelationshipType::ExpertIn),
        "LOCATED_IN" => Some(RelationshipType::LocatedIn),
        "PARTNERS_WITH" => Some(RelationshipType::PartnersWith),
        "RELATED_TO" => Some(RelationshipType::RelatedTo),
        "DEPENDS_ON" => Some(RelationshipType::DependsOn),
        "PART_OF" => Some(RelationshipType::PartOf),
        "FRIEND_OF" => Some(RelationshipType::FriendOf),
        "OWNS" => Some(RelationshipType::Owns),
        _ => None,
    }
}

/// Render an email into a compact string for the extraction prompt.
fn format_email_for_extraction(email: &EmailMessage) -> String {
    let mut parts = Vec::new();

    parts.push(format!("From: {}", email.from));
    if !email.to.is_empty() {
        parts.push(format!("To: {}", email.to.join(", ")));
    }
    if !email.cc.is_empty() {
        parts.push(format!("Cc: {}", email.cc.join(", ")));
    }
    parts.push(format!("Subject: {}", email.subject));
    parts.push(format!("Date: {}", email.date.to_rfc2822()));
    parts.push(String::new());

    if let Some(body) = &email.body_text {
        // Trim to 2000 chars to keep the prompt short and cost-efficient
        let truncated = if body.len() > 2000 {
            format!("{}...[truncated]", &body[..2000])
        } else {
            body.clone()
        };
        parts.push(truncated);
    } else if let Some(snippet) = &email.snippet {
        parts.push(snippet.clone());
    }

    parts.join("\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_config() -> ExtractorConfig {
        ExtractorConfig {
            base_url: "http://localhost".to_string(),
            api_key: "test-key".to_string(),
            model: "test-model".to_string(),
            max_tokens: 1024,
            confidence_threshold: 0.85,
            max_relationships: 3,
        }
    }

    fn make_email() -> EmailMessage {
        EmailMessage {
            id: "msg-001".to_string(),
            thread_id: "thread-001".to_string(),
            from: "user@example.com".to_string(),
            to: vec!["alice@example.com".to_string()],
            cc: vec![],
            bcc: vec![],
            subject: "Project update".to_string(),
            body_text: Some("Let's sync on Project Atlas this week.".to_string()),
            body_html: None,
            snippet: None,
            labels: vec!["SENT".to_string()],
            date: Utc::now(),
            is_sent: true,
            ingested_at: Utc::now(),
        }
    }

    fn make_user_context() -> UserContext {
        UserContext {
            user_id: "user-sub-001".to_string(),
            email: "user@example.com".to_string(),
            display_name: "Test User".to_string(),
        }
    }

    fn make_extractor() -> EntityExtractor {
        EntityExtractor::new(make_config())
    }

    // ------------------------------------------------------------------
    // parse_extraction_response tests
    // ------------------------------------------------------------------

    #[test]
    fn test_parse_entities_filters_low_confidence() {
        let extractor = make_extractor();
        let email = make_email();
        let ctx = make_user_context();

        let json = r#"{
            "entities": [
                {
                    "entity_type": "person",
                    "value": "Alice Smith",
                    "normalized": "alice_smith",
                    "confidence": 0.95,
                    "source": "header",
                    "context": "From: Alice Smith",
                    "aliases": []
                },
                {
                    "entity_type": "company",
                    "value": "Low Corp",
                    "normalized": "low_corp",
                    "confidence": 0.50,
                    "source": "body",
                    "context": null,
                    "aliases": []
                }
            ],
            "relationships": [],
            "overall_confidence": 0.80,
            "skipped_noise": false
        }"#;

        let result = extractor
            .parse_extraction_response(json, &email, &ctx, 100)
            .expect("parse should succeed");

        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.entities[0].value, "Alice Smith");
    }

    #[test]
    fn test_parse_entities_skips_person_from_body() {
        let extractor = make_extractor();
        let email = make_email();
        let ctx = make_user_context();

        let json = r#"{
            "entities": [
                {
                    "entity_type": "person",
                    "value": "Bob Jones",
                    "normalized": "bob_jones",
                    "confidence": 0.95,
                    "source": "body",
                    "context": "Bob Jones mentioned the deadline",
                    "aliases": []
                },
                {
                    "entity_type": "company",
                    "value": "Acme Corp",
                    "normalized": "acme_corp",
                    "confidence": 0.95,
                    "source": "body",
                    "context": "Acme Corp is the client",
                    "aliases": []
                }
            ],
            "relationships": [],
            "overall_confidence": 0.90,
            "skipped_noise": false
        }"#;

        let result = extractor
            .parse_extraction_response(json, &email, &ctx, 50)
            .expect("parse should succeed");

        // Bob Jones (person, body) must be filtered; Acme Corp (company, body) kept.
        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.entities[0].value, "Acme Corp");
    }

    #[test]
    fn test_parse_relationships_respects_max() {
        let extractor = make_extractor();
        let email = make_email();
        let ctx = make_user_context();

        // Four relationships provided — only top 3 by confidence should survive.
        let json = r#"{
            "entities": [],
            "relationships": [
                {
                    "from_entity_value": "alice_smith",
                    "from_entity_type": "person",
                    "to_entity_value": "acme_corp",
                    "to_entity_type": "company",
                    "relationship_type": "WORKS_FOR",
                    "confidence": 0.91,
                    "evidence": "Alice cc'd the Acme all-hands"
                },
                {
                    "from_entity_value": "bob_jones",
                    "from_entity_type": "person",
                    "to_entity_value": "project_atlas",
                    "to_entity_type": "project",
                    "relationship_type": "WORKS_ON",
                    "confidence": 0.88,
                    "evidence": "Bob is leading Project Atlas"
                },
                {
                    "from_entity_value": "carol_white",
                    "from_entity_type": "person",
                    "to_entity_value": "acme_corp",
                    "to_entity_type": "company",
                    "relationship_type": "WORKS_FOR",
                    "confidence": 0.86,
                    "evidence": "Carol at Acme"
                },
                {
                    "from_entity_value": "dan_black",
                    "from_entity_type": "person",
                    "to_entity_value": "project_atlas",
                    "to_entity_type": "project",
                    "relationship_type": "LEADS",
                    "confidence": 0.87,
                    "evidence": "Dan leads Atlas"
                }
            ],
            "overall_confidence": 0.88,
            "skipped_noise": false
        }"#;

        let result = extractor
            .parse_extraction_response(json, &email, &ctx, 200)
            .expect("parse should succeed");

        // Must cap at max_relationships (3) and pick highest confidence.
        assert_eq!(result.relationships.len(), 3);
        // Lowest confidence (carol at 0.86) is dropped; dan (0.87) kept.
        let dropped = result
            .relationships
            .iter()
            .any(|r| r.from_entity_value == "carol_white");
        assert!(!dropped, "carol_white (lowest conf) should be dropped");
    }

    #[test]
    fn test_skipped_noise_returns_empty() {
        let extractor = make_extractor();
        let email = make_email();
        let ctx = make_user_context();

        let json = r#"{
            "entities": [],
            "relationships": [],
            "overall_confidence": 1.0,
            "skipped_noise": true
        }"#;

        let result = extractor
            .parse_extraction_response(json, &email, &ctx, 10)
            .expect("parse should succeed");

        assert!(result.skipped_noise);
        assert!(result.entities.is_empty());
        assert!(result.relationships.is_empty());
    }

    // ------------------------------------------------------------------
    // is_noise_email tests
    // ------------------------------------------------------------------

    #[test]
    fn test_is_noise_email_detects_noreply() {
        let mut email = make_email();
        email.from = "noreply@github.com".to_string();
        assert!(is_noise_email(&email));
    }

    #[test]
    fn test_is_noise_email_detects_no_reply_variant() {
        let mut email = make_email();
        email.from = "no-reply@notifications.slack.com".to_string();
        assert!(is_noise_email(&email));
    }

    #[test]
    fn test_is_noise_email_detects_newsletter_subject() {
        let mut email = make_email();
        email.subject = "Weekly Newsletter — Issue #42".to_string();
        assert!(is_noise_email(&email));
    }

    #[test]
    fn test_is_noise_email_detects_jira_subject() {
        let mut email = make_email();
        email.subject = "[JIRA] Issue TRS-123 was updated".to_string();
        assert!(is_noise_email(&email));
    }

    #[test]
    fn test_is_noise_email_detects_promotions_label() {
        let mut email = make_email();
        email.labels = vec!["CATEGORY_PROMOTIONS".to_string()];
        assert!(is_noise_email(&email));
    }

    #[test]
    fn test_is_noise_email_clean_email_passes() {
        let email = make_email();
        assert!(!is_noise_email(&email));
    }
}
