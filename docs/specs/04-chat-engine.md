# Chat Engine

## Overview

The chat engine is the user-facing brain of trusty-izzie. It orchestrates: session management, context assembly from the knowledge graph and memory system, LLM interaction via OpenRouter, tool execution, and persistence of new memories discovered in the conversation.

The engine is implemented in `trusty-chat`. It is designed to work identically whether consumed via the HTTP API (SSE streaming), Unix socket IPC (CLI), or TUI (direct function calls).

---

## Session Management

Sessions are stored in SQLite. Each session has a sliding window of recent messages and a compressed summary of older context.

### Session Struct

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id:              String,        // UUID v4
    pub user_id:         String,
    pub title:           Option<String>, // auto-generated from first message
    pub created_at:      DateTime<Utc>,
    pub last_active_at:  DateTime<Utc>,
    pub message_count:   u32,
    pub compressed_summary: Option<String>, // compressed older context
    pub token_estimate:  u32,           // rolling token count estimate
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id:         String,       // UUID v4
    pub session_id: String,
    pub role:       MessageRole,  // User | Assistant | Tool
    pub content:    String,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_results: Option<Vec<ToolResult>>,
    pub created_at: DateTime<Utc>,
    pub tokens:     Option<u32>,  // token count if known
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
}
```

### Sliding Window Compression

When a session's estimated token count exceeds a configurable threshold (default: 6000 tokens), the engine compresses the oldest messages:

```rust
const COMPRESSION_TRIGGER_TOKENS: u32 = 6000;
const WINDOW_KEEP_MESSAGES: usize = 10;  // Always keep the last 10 messages verbatim

pub async fn compress_session(
    session_id: &str,
    store: &Store,
    llm_client: &LlmClient,
) -> TrustyResult<()> {
    let messages = store.sessions().get_messages(session_id).await?;
    if messages.len() <= WINDOW_KEEP_MESSAGES {
        return Ok(());
    }

    let to_compress = &messages[..messages.len() - WINDOW_KEEP_MESSAGES];

    let summary_prompt = format!(
        "Summarize the following conversation history concisely. \
        Preserve key facts, decisions, and context that would help continue the conversation. \
        Be brief but complete.\n\n{}",
        format_messages_for_compression(to_compress)
    );

    let summary = llm_client.complete_simple(&summary_prompt).await?;

    store.sessions().update_compressed_summary(session_id, &summary).await?;
    store.sessions().delete_messages(session_id, &to_compress.iter().map(|m| m.id.clone()).collect::<Vec<_>>()).await?;

    Ok(())
}
```

---

## System Prompt Assembly

The system prompt is rebuilt on every turn. It includes:
1. Assistant identity and role
2. Current date/time
3. Entity context (from hybrid search over knowledge graph)
4. Memory context (from memory retrieval)
5. Tool definitions
6. Response format instructions

### Full System Prompt Template

```rust
pub fn build_system_prompt(
    context: &AssembledContext,
    config: &ChatConfig,
) -> String {
    let entity_section = if context.entities.is_empty() {
        String::new()
    } else {
        format!(
            "## People, Companies, and Projects I know about:\n{}",
            context.entities.iter().enumerate().map(|(i, e)| {
                format!("{}. {} ({}): {}", i+1, e.name, e.entity_type, e.summary)
            }).collect::<Vec<_>>().join("\n")
        )
    };

    let memory_section = format_memories_for_context(&context.memories);

    let calendar_section = if context.calendar_events.is_empty() {
        String::new()
    } else {
        format!(
            "## My calendar (next 7 days):\n{}",
            context.calendar_events.iter().map(|e| {
                format!("- {}: {} ({})", e.date, e.title, e.time_or_allday)
            }).collect::<Vec<_>>().join("\n")
        )
    };

    format!(r#"You are trusty-izzie, a personal AI assistant with deep knowledge of the user's professional relationships and work context. You are running locally on the user's machine.

Today is {today}. Current time: {time}.

{entity_section}

{memory_section}

{calendar_section}

## Response format
You MUST respond with a JSON object in this exact format:
{{
  "message": "your response to the user (markdown allowed)",
  "currentTask": "brief description of what you just did (1 sentence)",
  "memoriesToSave": [
    {{
      "content": "memory text, max 500 chars",
      "category": "preference|fact|relationship|decision|event|sentiment|reminder",
      "confidence": 0.0-1.0,
      "importance": 0.0-1.0,
      "entity_refs": ["entity-id-1", "entity-id-2"]
    }}
  ]
}}

Only include items in memoriesToSave if you learned something genuinely new and useful from this turn. Do not re-save information already in the context above. Be selective — 0-2 memories per turn is typical.

## Cost awareness
You are running on OpenRouter with real API costs. Be thorough but avoid unnecessary verbosity. Use tools when they would meaningfully improve your answer; don't call tools for information already in context.

## Tool use
Call tools when the user asks about specific relationships, entities, or information that might be in the knowledge graph but isn't in the context above. Do not call the same tool twice with the same arguments.
"#,
        today = context.now.format("%A, %B %d, %Y"),
        time = context.now.format("%H:%M"),
        entity_section = entity_section,
        memory_section = memory_section,
        calendar_section = calendar_section,
    )
}
```

---

## Context Assembly

Before building the system prompt, context is assembled via hybrid search:

```rust
pub async fn assemble_context(
    query: &str,
    store: &Store,
    engine: &EmbeddingEngine,
    search: &HybridSearchEngine,
    memory_manager: &MemoryManager,
) -> TrustyResult<AssembledContext> {
    // Embed the user's query once, reuse for both entity + memory search
    let query_vec = engine.embed_one(query).await?;

    // Parallel: entity search + memory retrieval
    let (entity_results, memory_results, calendar_events) = tokio::try_join!(
        search_entities(query, &query_vec, store, search),
        memory_manager.retrieve_context(query, 8, store, engine, search),
        store.sessions().get_calendar_cache(),
    )?;

    Ok(AssembledContext {
        entities: entity_results.into_iter().take(10).collect(),
        memories: memory_results,
        calendar_events,
        now: Utc::now(),
    })
}

async fn search_entities(
    query: &str,
    query_vec: &[f32],
    store: &Store,
    search: &HybridSearchEngine,
) -> TrustyResult<Vec<ContextEntity>> {
    // BM25 search
    let bm25 = store.entities().bm25_search(query, 15).await?;
    // Vector search
    let vector = store.entities().vector_search(query_vec.to_vec(), 15).await?;
    // Fuse
    let fused = search.fuse(&vector, &bm25, 0.7, 60);
    // Fetch full entity details for top results
    let mut entities = Vec::new();
    for result in fused.iter().take(10) {
        if let Some(entity) = store.entities().get_by_id(&result.id).await? {
            // Get graph neighbors for summary enrichment
            let neighbors = store.graph().get_neighbors(&entity.id, 1).await?;
            entities.push(ContextEntity::from_entity_and_neighbors(entity, neighbors));
        }
    }
    Ok(entities)
}
```

---

## Tool Call Loop

The engine runs up to 5 iterations of the tool call loop. This prevents infinite loops while allowing complex multi-step queries.

```rust
pub async fn run_tool_loop(
    session: &ChatSession,
    user_message: &str,
    system_prompt: &str,
    store: &Store,
    llm: &LlmClient,
    tools: &ToolRegistry,
    tx: &mpsc::Sender<ChatEvent>,
) -> TrustyResult<ChatTurn> {
    const MAX_ITERATIONS: u8 = 5;

    let mut messages = build_initial_messages(session, user_message, system_prompt);
    let mut iterations = 0u8;
    let mut total_tokens = 0u32;
    let mut tool_calls_made = Vec::new();

    loop {
        iterations += 1;
        if iterations > MAX_ITERATIONS {
            // Force final answer without tools
            messages.push(OpenRouterMessage {
                role: "user".into(),
                content: "Please provide your final answer now.".into(),
            });
        }

        // Call LLM (streaming)
        let (response_text, usage) = llm.stream_with_tools(
            &messages,
            if iterations <= MAX_ITERATIONS { tools.schemas() } else { &[] },
            tx,
        ).await?;

        total_tokens += usage.total_tokens;

        // Parse structured response
        let parsed = parse_llm_response(&response_text)?;

        if let Some(tool_call) = parsed.tool_call {
            // Execute tool
            tx.send(ChatEvent::ToolCall(ToolCallEvent { name: tool_call.name.clone(), args: tool_call.args.clone() })).await?;
            let result = tools.dispatch(&tool_call, store).await?;
            tx.send(ChatEvent::ToolResult(ToolResultEvent { result: result.clone() })).await?;

            // Add tool call + result to message history
            messages.push(OpenRouterMessage::assistant_with_tool_call(&tool_call));
            messages.push(OpenRouterMessage::tool_result(&tool_call.id, &result));

            tool_calls_made.push(tool_call.name.clone());
        } else {
            // No tool call — we have the final answer
            let turn = ChatTurn {
                message: parsed.message,
                current_task: parsed.current_task,
                memories_to_save: parsed.memories_to_save,
                tool_calls_made,
                tokens_used: total_tokens,
            };
            tx.send(ChatEvent::Done(turn.clone())).await?;
            return Ok(turn);
        }
    }
    unreachable!()
}
```

---

## Structured Response Format

All LLM responses MUST conform to this JSON schema:

```json
{
  "$schema": "http://json-schema.org/draft-07/schema",
  "type": "object",
  "required": ["message", "currentTask", "memoriesToSave"],
  "properties": {
    "message": {
      "type": "string",
      "description": "The response to show the user. Markdown is rendered in TUI."
    },
    "currentTask": {
      "type": "string",
      "description": "One-sentence description of what the assistant just did."
    },
    "memoriesToSave": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["content", "category", "confidence", "importance"],
        "properties": {
          "content":     { "type": "string", "maxLength": 500 },
          "category":    { "type": "string", "enum": ["preference","fact","relationship","decision","event","sentiment","reminder"] },
          "confidence":  { "type": "number", "minimum": 0.0, "maximum": 1.0 },
          "importance":  { "type": "number", "minimum": 0.0, "maximum": 1.0 },
          "entity_refs": { "type": "array", "items": { "type": "string" } }
        }
      }
    }
  }
}
```

**Parsing with fallback:**
```rust
pub fn parse_llm_response(raw: &str) -> TrustyResult<ParsedResponse> {
    // LLMs sometimes wrap JSON in markdown code blocks
    let cleaned = strip_markdown_fences(raw);

    serde_json::from_str::<ParsedResponse>(&cleaned).or_else(|_| {
        // Fallback: treat entire response as the message field
        // This handles cases where the model refuses to output JSON
        Ok(ParsedResponse {
            message: raw.to_string(),
            current_task: "responded to user".to_string(),
            memories_to_save: vec![],
            tool_call: None,
        })
    })
}
```

---

## Streaming

### SSE (HTTP API)

```rust
// In trusty-api route handler
pub async fn chat_handler(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel::<ChatEvent>(32);

    tokio::spawn(async move {
        state.chat_engine
            .send_message(&req.session_id, &req.message, &state.store, tx)
            .await
            .unwrap_or_else(|e| tracing::error!("Chat error: {e}"));
    });

    let stream = ReceiverStream::new(rx).map(|event| {
        let data = serde_json::to_string(&event).unwrap_or_default();
        Ok(Event::default().data(data))
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
```

SSE event types (all serialized as JSON in the `data` field):
```
event: token        → { "token": "Hello" }
event: tool_call    → { "name": "entity_query", "args": {...} }
event: tool_result  → { "result": {...} }
event: done         → { "message": "...", "tokens_used": 342, ... }
event: error        → { "error": "..." }
```

### CLI / IPC

For CLI and TUI consumers, tokens are printed/rendered as they arrive from the channel without SSE wrapping:

```rust
// trusty-cli chat command
let mut rx = engine.send_message(session_id, message, &store, opts).await?;
while let Some(event) = rx.recv().await {
    match event {
        ChatEvent::Token(t)    => print!("{t}"),
        ChatEvent::Done(turn)  => {
            println!();
            if !turn.memories_to_save.is_empty() {
                eprintln!("[{} memories saved]", turn.memories_to_save.len());
            }
        }
        ChatEvent::ToolCall(tc) => eprintln!("[calling tool: {}]", tc.name),
        _ => {}
    }
}
```

---

## Tool Definitions

### Tool: `entity_query`

Search the knowledge graph for entities matching a description.

```json
{
  "name": "entity_query",
  "description": "Search for people, companies, projects, tools, or topics in the knowledge graph",
  "parameters": {
    "type": "object",
    "required": ["query"],
    "properties": {
      "query": { "type": "string", "description": "Natural language search query" },
      "entity_type": {
        "type": "string",
        "enum": ["Person", "Company", "Project", "Tool", "Topic", "Location"],
        "description": "Filter by entity type (optional)"
      },
      "limit": { "type": "integer", "default": 5, "maximum": 20 }
    }
  }
}
```

**Execution:** Runs hybrid search (BM25 + vector) over `{user_id}_entities` LanceDB table.

---

### Tool: `find_related`

Find entities related to a given entity via the graph.

```json
{
  "name": "find_related",
  "description": "Find entities related to a specific entity in the knowledge graph",
  "parameters": {
    "type": "object",
    "required": ["entity_id"],
    "properties": {
      "entity_id": { "type": "string", "description": "ID of the entity to find neighbors for" },
      "relationship_type": {
        "type": "string",
        "description": "Filter by relationship type (optional)",
        "enum": ["WORKS_FOR","WORKS_WITH","WORKS_ON","REPORTS_TO","LEADS","EXPERT_IN","LOCATED_IN","PARTNERS_WITH","RELATED_TO"]
      },
      "depth": { "type": "integer", "default": 1, "maximum": 2 }
    }
  }
}
```

**Execution:** Runs Kuzu Cypher query.

---

### Tool: `memory_search`

Search the memory store for relevant episodic memories.

```json
{
  "name": "memory_search",
  "description": "Search stored memories and past observations about people or topics",
  "parameters": {
    "type": "object",
    "required": ["query"],
    "properties": {
      "query": { "type": "string" },
      "category": {
        "type": "string",
        "enum": ["preference","fact","relationship","decision","event","sentiment","reminder"]
      },
      "limit": { "type": "integer", "default": 5, "maximum": 15 }
    }
  }
}
```

---

### Tool: `email_search`

Search processed email summaries (not raw email content — only extracted metadata).

```json
{
  "name": "email_search",
  "description": "Search processed email context for mentions of people, projects, or topics",
  "parameters": {
    "type": "object",
    "required": ["query"],
    "properties": {
      "query": { "type": "string" },
      "from_date": { "type": "string", "description": "ISO date, e.g. 2025-01-01" },
      "to_date":   { "type": "string", "description": "ISO date" },
      "limit":     { "type": "integer", "default": 5 }
    }
  }
}
```

**Note:** This searches the entity extraction results (stored in LanceDB), not raw Gmail content. The assistant never re-reads raw emails.

---

### Tool: `calendar_today`

Get the user's calendar events for today or a specific day.

```json
{
  "name": "calendar_today",
  "description": "Get calendar events for today or a specific date",
  "parameters": {
    "type": "object",
    "properties": {
      "date": {
        "type": "string",
        "description": "ISO date (YYYY-MM-DD). Defaults to today."
      }
    }
  }
}
```

**Execution:** Reads from calendar cache in SQLite (refreshed every 30 min by daemon).

---

## LLM Client Configuration

```rust
pub struct LlmClient {
    client:    reqwest::Client,
    api_key:   String,
    base_url:  String,  // "https://openrouter.ai/api/v1"
    chat_model: String, // "anthropic/claude-sonnet-4-5" (default)
    extract_model: String, // "mistralai/mistral-small" (entity extraction)
}

pub struct LlmOptions {
    pub max_tokens:   u32,       // default: 2048
    pub temperature:  f32,       // default: 0.3 for extraction, 0.7 for chat
    pub stream:       bool,      // default: true for chat, false for extraction
    pub response_format: Option<ResponseFormat>, // JSON mode when available
}
```

**OpenRouter API request format:**
```json
{
  "model": "anthropic/claude-sonnet-4-5",
  "messages": [...],
  "tools": [...],
  "stream": true,
  "max_tokens": 2048,
  "temperature": 0.7
}
```

---

## Session Lifecycle

```
User sends first message
         │
         ▼
ChatEngine::new_session() → INSERT into chat_sessions
         │
         ▼
ChatEngine::send_message()
   │
   ├── assemble_context(query)
   │     ├── embed query
   │     ├── hybrid search entities
   │     └── retrieve memories
   │
   ├── build_system_prompt(context)
   │
   ├── load_session_messages() → last N messages + compressed_summary
   │
   ├── run_tool_loop() → stream tokens
   │
   ├── persist new memories
   │
   ├── INSERT message into chat_messages
   │
   └── UPDATE session.last_active_at, token_estimate
         │
         ▼
   If token_estimate > COMPRESSION_TRIGGER:
         └── compress_session() (async, non-blocking)
```

---

## Chat Options

```rust
pub struct ChatOptions {
    pub model:        Option<String>,  // override default chat model
    pub max_tokens:   Option<u32>,
    pub temperature:  Option<f32>,
    pub disable_tools: bool,           // raw chat mode
    pub disable_memory_save: bool,     // don't persist new memories
}
```
