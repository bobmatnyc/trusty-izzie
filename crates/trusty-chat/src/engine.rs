//! The core chat completion engine.

use anyhow::Result;
use uuid::Uuid;

use trusty_models::chat::{ChatMessage, ChatSession, MessageRole, StructuredResponse};

/// Drives the conversation loop: context assembly → LLM call → tool dispatch → save.
#[allow(dead_code)]
pub struct ChatEngine {
    http: reqwest::Client,
    api_base: String,
    api_key: String,
    model: String,
    max_tool_iterations: u32,
}

impl ChatEngine {
    /// Construct the chat engine.
    pub fn new(api_base: String, api_key: String, model: String, max_tool_iterations: u32) -> Self {
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
        // Append the user message
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

        // TODO:
        // 1. Assemble RAG context (recall memories + entities)
        // 2. Build message list with system prompt
        // 3. Call OpenRouter (up to max_tool_iterations for tool calls)
        // 4. Parse StructuredResponse, save memories_to_save
        // 5. Append assistant message to session

        todo!("implement full chat completion loop with tool dispatch")
    }
}
