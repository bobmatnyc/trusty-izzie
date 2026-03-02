//! The core chat completion engine.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use trusty_models::chat::{ChatMessage, ChatSession, MessageRole, StructuredResponse};

use crate::context::ContextAssembler;

/// Drives the conversation loop: context assembly → LLM call → tool dispatch → save.
pub struct ChatEngine {
    http: reqwest::Client,
    api_base: String,
    api_key: String,
    model: String,
    /// Reserved for the tool call loop (phase 2 — not yet implemented).
    #[allow(dead_code)]
    max_tool_iterations: u32,
    context_assembler: ContextAssembler,
}

// ── OpenRouter request/response types ────────────────────────────────────────

#[derive(Serialize)]
struct OrchatRequest<'a> {
    model: &'a str,
    messages: Vec<OrchatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
    max_tokens: u32,
    temperature: f32,
    response_format: ResponseFormat,
}

#[derive(Serialize, Deserialize, Clone)]
struct OrchatMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ResponseFormat {
    r#type: &'static str,
}

#[derive(Deserialize)]
struct OrchatResponse {
    choices: Vec<OrchatChoice>,
    usage: Option<OrchatUsage>,
}

#[derive(Deserialize)]
struct OrchatChoice {
    message: OrchatAssistantMsg,
}

#[derive(Deserialize)]
struct OrchatAssistantMsg {
    content: String,
}

#[derive(Deserialize)]
struct OrchatUsage {
    total_tokens: u32,
}

// ── ChatEngine impl ───────────────────────────────────────────────────────────

impl ChatEngine {
    /// Construct the chat engine with a default (empty) context assembler.
    pub fn new(api_base: String, api_key: String, model: String, max_tool_iterations: u32) -> Self {
        Self::new_with_context(
            api_base,
            api_key,
            model,
            max_tool_iterations,
            ContextAssembler::new(5, 10),
        )
    }

    /// Construct the chat engine with a fully configured context assembler.
    pub fn new_with_context(
        api_base: String,
        api_key: String,
        model: String,
        max_tool_iterations: u32,
        context_assembler: ContextAssembler,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("failed to build HTTP client");
        Self {
            http,
            api_base,
            api_key,
            model,
            max_tool_iterations,
            context_assembler,
        }
    }

    /// Process a single user turn, returning the assistant's reply.
    ///
    /// The caller is responsible for loading and saving the session.
    pub async fn chat(
        &self,
        session: &mut ChatSession,
        user_message: &str,
    ) -> Result<StructuredResponse> {
        // 1. Append user message
        session.messages.push(ChatMessage {
            id: Uuid::new_v4(),
            session_id: session.id,
            role: MessageRole::User,
            content: user_message.to_string(),
            tool_name: None,
            tool_result: None,
            token_count: None,
            created_at: chrono::Utc::now(),
        });

        // 2. Assemble RAG context from LanceDB entities and memories
        let ctx = self.context_assembler.assemble(user_message, "").await?;
        let context_section = self.context_assembler.render_context(&ctx);

        // 3. Build messages for OpenRouter
        let now = chrono::Utc::now();
        let system_content = system_prompt(now, &context_section);

        let mut messages: Vec<OrchatMessage> = vec![OrchatMessage {
            role: "system".to_string(),
            content: system_content,
        }];

        for msg in &session.messages {
            let role = match msg.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::Tool => "tool",
                MessageRole::System => "system",
            };
            messages.push(OrchatMessage {
                role: role.to_string(),
                content: msg.content.clone(),
            });
        }

        // 4. Call OpenRouter
        let url = format!("{}/chat/completions", self.api_base);
        let request_body = OrchatRequest {
            model: &self.model,
            messages,
            tools: None,
            max_tokens: 2048,
            temperature: 0.7,
            response_format: ResponseFormat {
                r#type: "json_object",
            },
        };

        let http_response = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&request_body)
            .send()
            .await
            .context("failed to send request to OpenRouter")?;

        let status = http_response.status();
        if !status.is_success() {
            let body = http_response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable>".to_string());
            return Err(anyhow::anyhow!("OpenRouter returned {}: {}", status, body));
        }

        let or_response: OrchatResponse = http_response
            .json()
            .await
            .context("failed to deserialize OpenRouter response")?;

        let raw_content = or_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default();

        let token_count = or_response.usage.map(|u| u.total_tokens);

        // 5. Parse StructuredResponse with fallback; clean trailing JSON fence from reply.
        let mut structured = parse_response(&raw_content);
        structured.reply = clean_reply(&structured.reply);

        // 6. Append assistant message
        session.messages.push(ChatMessage {
            id: Uuid::new_v4(),
            session_id: session.id,
            role: MessageRole::Assistant,
            content: structured.reply.clone(),
            tool_name: None,
            tool_result: None,
            token_count,
            created_at: chrono::Utc::now(),
        });

        Ok(structured)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn system_prompt(now: chrono::DateTime<chrono::Utc>, context: &str) -> String {
    let context_section = if context.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", context)
    };
    format!(
        r#"You are trusty-izzie, a personal AI assistant with deep knowledge of the user's professional relationships and work context. You run locally on the user's machine.

Today is {}. Current time: {}.{context_section}

CRITICAL OUTPUT FORMAT: Your ENTIRE response must be a single raw JSON object. Output ONLY the JSON — no prose before it, no explanation after it, no markdown code fences around it. Start your response with {{ and end with }}.

Required format (output this and nothing else):
{{"reply":"your response to the user (markdown allowed)","memoriesToSave":[],"referencedEntities":[]}}

Be helpful, concise, and honest. Only include items in memoriesToSave if you learned something genuinely new and useful. Be selective — 0-1 memories per turn is typical."#,
        now.format("%A, %B %d, %Y"),
        now.format("%H:%M UTC"),
    )
}

/// Strip ```json ... ``` or ``` ... ``` fences if present.
fn strip_fences(raw: &str) -> &str {
    let trimmed = raw.trim();
    // Try ```json first, then plain ```
    for prefix in &["```json", "```"] {
        if let Some(after_open) = trimmed.strip_prefix(prefix) {
            if let Some(stripped) = after_open.strip_suffix("```") {
                return stripped.trim();
            }
        }
    }
    trimmed
}

fn parse_response(raw: &str) -> StructuredResponse {
    // 1. Try the whole response (after stripping outer fences)
    let cleaned = strip_fences(raw);
    if let Ok(s) = serde_json::from_str::<StructuredResponse>(cleaned) {
        return s;
    }

    // 2. Model sometimes emits text preamble then a ```json ... ``` block.
    //    Search for the LAST ```json block in the response.
    let mut last_fence_pos = None;
    let mut search_from = 0;
    while let Some(pos) = raw[search_from..].find("```json") {
        last_fence_pos = Some(search_from + pos);
        search_from += pos + 7; // 7 = len("```json")
    }
    if let Some(fence_start) = last_fence_pos {
        let after_open = &raw[fence_start + 7..]; // skip "```json"
        if let Some(close) = after_open.find("```") {
            let inner = after_open[..close].trim();
            if let Ok(s) = serde_json::from_str::<StructuredResponse>(inner) {
                return s;
            }
        }
    }

    // 3. Try to find any valid JSON object starting with '{' that has a "reply" field.
    let mut search = raw;
    while let Some(start) = search.find('{') {
        let candidate = &search[start..];
        if let Ok(s) = serde_json::from_str::<StructuredResponse>(candidate) {
            return s;
        }
        search = &search[start + 1..];
    }

    // 4. Fallback: treat the whole raw string as a plain-text reply.
    StructuredResponse {
        reply: raw.to_string(),
        memories_to_save: vec![],
        referenced_entities: vec![],
    }
}

/// Remove any trailing ```json ... ``` block that the model sometimes appends
/// inside the reply field as a "structured output" summary.
fn clean_reply(reply: &str) -> String {
    // Look for the LAST occurrence of ```json or ``` in the trimmed reply.
    // If found, strip from that point onward (and any trailing whitespace before it).
    let trimmed = reply.trim_end();
    for fence in &["```json", "```"] {
        if let Some(pos) = trimmed.rfind(fence) {
            // Only strip if the fence appears after a newline (not inline code)
            if pos == 0 || trimmed.as_bytes().get(pos - 1) == Some(&b'\n') {
                return trimmed[..pos].trim_end().to_string();
            }
        }
    }
    trimmed.to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_response_valid_json() {
        let json = r#"{"reply":"Hello there!","memoriesToSave":[],"referencedEntities":[]}"#;
        let result = parse_response(json);
        assert_eq!(result.reply, "Hello there!");
        assert!(result.memories_to_save.is_empty());
        assert!(result.referenced_entities.is_empty());
    }

    #[test]
    fn test_parse_response_fallback_on_bad_json() {
        let raw = "This is not JSON at all.";
        let result = parse_response(raw);
        assert_eq!(result.reply, raw);
        assert!(result.memories_to_save.is_empty());
    }

    #[test]
    fn test_parse_response_strips_markdown_fences() {
        let fenced =
            "```json\n{\"reply\":\"Hi!\",\"memoriesToSave\":[],\"referencedEntities\":[]}\n```";
        let result = parse_response(fenced);
        assert_eq!(result.reply, "Hi!");
    }

    #[test]
    fn test_system_prompt_contains_date() {
        let now = chrono::Utc::now();
        let prompt = system_prompt(now, "");
        let year = now.format("%Y").to_string();
        assert!(prompt.contains(&year));
    }

    #[test]
    fn test_system_prompt_includes_context_when_nonempty() {
        let now = chrono::Utc::now();
        let prompt = system_prompt(
            now,
            "## Relevant People & Projects\n- Alice (Person): alice",
        );
        assert!(prompt.contains("## Relevant People & Projects"));
    }

    #[test]
    fn test_system_prompt_no_context_section_when_empty() {
        let now = chrono::Utc::now();
        let prompt = system_prompt(now, "");
        assert!(!prompt.contains("## Relevant"));
    }

    #[test]
    fn test_strip_fences_plain_backticks() {
        let fenced = "```\n{\"key\":\"value\"}\n```";
        let result = strip_fences(fenced);
        assert_eq!(result, "{\"key\":\"value\"}");
    }

    #[test]
    fn test_strip_fences_no_fences() {
        let raw = r#"{"key":"value"}"#;
        assert_eq!(strip_fences(raw), raw);
    }
}
