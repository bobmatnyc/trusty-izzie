//! The `EntityExtractor` struct and its extraction logic.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use trusty_models::email::EmailMessage;

use crate::prompt::EXTRACTION_PROMPT;
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

    /// Parse the raw JSON string returned by the LLM into an `ExtractionResult`.
    fn parse_extraction_response(
        &self,
        json_str: &str,
        email: &EmailMessage,
        user_context: &UserContext,
        tokens_used: u32,
    ) -> Result<ExtractionResult> {
        // We parse into a loosely-typed Value first to handle partial outputs
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

        // TODO: deserialise entities and relationships from `raw`, applying
        //       confidence_threshold and max_relationships filters.
        // For now, return a stub result so the binary compiles.
        let _ = user_context;
        todo!("deserialise entities/relationships from raw JSON and apply filters")
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
