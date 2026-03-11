# Spec 10: MCP Service for trusty-izzie ("Izzie as an MCP Server")

**Status:** Design
**Date:** 2026-03-11
**Author:** Research / Architecture

---

## 1. Executive Summary

This document specifies a new crate `crates/trusty-mcp/` that exposes trusty-izzie's personal-context data as an MCP (Model Context Protocol) server. External AI clients — Claude Desktop, Cursor, Zed, and any MCP-compatible tool — will be able to call Izzie to retrieve calendar events, tasks, contacts, memories, entities, and schedule new events without any new ports or OAuth flows.

---

## 2. Codebase Investigation Findings

### 2.1 Existing tool surface (trusty-chat)

`crates/trusty-chat/src/tools.rs` defines 35 tool variants in `ToolName`. The engine in `engine.rs` implements each as an `async fn` that returns `Result<String>`. All tool implementations live on the `ChatEngine` struct, which holds:

- `reqwest::Client` — for Google API calls (Calendar, Tasks)
- `SqliteStore` (via `Arc`) — for OAuth tokens, event queue, accounts, preferences, open loops
- `PathBuf` (agents_dir) — for agent Markdown definitions
- Direct `rusqlite::Connection` opens for iMessage, AddressBook, and WhatsApp reads

The key insight: **tool logic is self-contained in `ChatEngine` methods**. No separate service layer exists yet. The API handlers in `crates/trusty-api/src/handlers/` for memories and entities are `todo!()` stubs — they are not yet wired up to LanceDB or Kuzu.

### 2.2 REST API (`crates/trusty-api`)

- Port 3457, host 127.0.0.1 (from `config/default.toml`)
- Routes: `/health`, `/chat`, `/chat/sessions`, `/entities`, `/memories`, `/sync`, `/api/agents`, `/api/tasks`
- `AppState`: `Arc<AppConfig>` + `Arc<SqliteStore>` + `PathBuf` (agents_dir)
- Built on axum 0.7

### 2.3 Storage layer (`crates/trusty-store`)

Three backends, re-exported from `lib.rs`:
- `LanceStore` — vector search over memories and entities (LanceDB / Arrow)
- `GraphStore` — Kuzu graph for entity/relationship traversal
- `SqliteStore` — relational: OAuth tokens, events, accounts, preferences, open loops, agent tasks

`SqliteStore` holds everything needed for tasks, calendar access (via OAuth tokens), event queue scheduling, and account listing. This is the primary dependency for the MCP server.

### 2.4 Dependencies available in workspace

```toml
tokio = { version = "1", features = ["full"] }
axum = { version = "0.7", features = ["ws", "macros"] }
serde / serde_json = "1"
reqwest = { version = "0.12", features = ["json"] }
rusqlite = { version = "0.31", features = ["bundled"] }
chrono = "0.4"
anyhow / thiserror = "1"
tracing / tracing-subscriber = "0.1" / "0.3"
clap = { version = "4", features = ["derive", "env"] }
```

No MCP SDK exists in the workspace — it must be added.

---

## 3. Architecture Decision: Transport

### 3.1 Options evaluated

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| **stdio only** | Binary spawned by client, JSON-RPC on stdin/stdout | Zero extra ports, Claude Desktop native, simple deployment | No HTTP clients, every client spawns a new process |
| **HTTP SSE only** | Persistent server, `/mcp/sse` endpoint | Shared state, any HTTP client, Cursor/Zed friendly | Additional port, always-on process |
| **Both (recommended)** | Same binary, `--stdio` flag or HTTP SSE via trusty-api | Covers all clients, one implementation | Slightly more code |

### 3.2 Decision: Both transports in a single binary

The `trusty-mcp` binary will support:

1. **stdio** (flag: `--stdio`): Spawned by Claude Desktop. Reads JSON-RPC from stdin, writes to stdout. This is the simplest integration path and required for Claude Desktop.

2. **HTTP SSE** (default, no flag): Runs a small axum HTTP server on a configurable port (default 3458 — separate from trusty-api's 3457). Claude Desktop can also use this via `"url"` config. Cursor, Zed, and any other HTTP-capable client uses this.

The **tool handler code is shared**. Both transports call the same `McpServer` struct, which owns a `ChatEngine` (reused from `trusty-chat`) plus direct SQLite access.

### 3.3 Rationale for not adding MCP to trusty-api

trusty-api is a REST API for Izzie's own frontend/TUI. Adding SSE there would mix concerns and require LanceDB/Kuzu to be started (they are lazy in the daemon). The MCP server only needs `SqliteStore` and `reqwest` to serve its tools — it is lighter than trusty-api.

---

## 4. MCP Tools Exposed

The following tools are selected from the 35 available in `ToolName`. Selection criteria: useful for external AI orchestration, read-mostly (or clearly bounded write), does not require macOS permissions that an external process cannot obtain (e.g., Full Disk Access for iMessage/WhatsApp is excluded by default).

### 4.1 Tool Catalog

| MCP Tool Name | Izzie ToolName | Category | Notes |
|---------------|----------------|----------|-------|
| `search_contacts` | `SearchContacts` | Contacts | AddressBook read — requires Full Disk Access on the machine running trusty-mcp |
| `get_calendar_events` | `GetCalendarEvents` | Calendar | Google Calendar via OAuth token in SQLite |
| `create_calendar_event` | `CreateCalendarEvent` | Calendar | Creates event on Google Calendar |
| `get_tasks` | `GetTasks` | Tasks | Google Tasks for a specific list |
| `get_tasks_bulk` | `GetTasksBulk` | Tasks | All lists + tasks for an account in one call |
| `search_memories` | `SearchMemories` | Memory | Semantic search over LanceDB memories |
| `search_entities` | `SearchEntities` | Knowledge | Entity search (LanceDB + Tantivy BM25) |
| `get_entity_relationships` | `GetEntityRelationships` | Knowledge | Kuzu graph traversal |
| `get_context` | _(composite)_ | Context | Composite: memories + entities + calendar summary for a named person/topic |
| `list_accounts` | `ListAccounts` | Accounts | Which Google accounts are connected |
| `schedule_event` | `ScheduleEvent` | Scheduling | Enqueue a reminder or system event |
| `list_open_loops` | `ListOpenLoops` | Follow-ups | Pending items awaiting follow-up |
| `get_preferences` | `GetPreferences` | Settings | User preferences (proactive features) |

**Excluded from initial release** (security-sensitive or require system permissions not suitable for external orchestration):
- `ExecuteShellCommand` — arbitrary shell execution, not safe to expose
- `SearchImessages`, `SearchWhatsapp` — Full Disk Access required; can be opt-in later
- `SyncContacts`, `SyncMessages`, `SyncWhatsApp` — side-effectful sync ops
- `AddAccount`, `RemoveAccount` — account management should stay in-app
- `SubmitGithubIssue` — requires `gh` CLI, niche use
- `RunAgent`, `GetAgentTask` — agent orchestration; future v2

### 4.2 JSON Schema Definitions

#### `search_contacts`
```json
{
  "name": "search_contacts",
  "description": "Search your macOS AddressBook contacts by name, email, or phone number",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "description": "Name, email, or phone to search for" },
      "limit": { "type": "integer", "default": 10, "minimum": 1, "maximum": 50 }
    },
    "required": ["query"]
  }
}
```

#### `get_calendar_events`
```json
{
  "name": "get_calendar_events",
  "description": "Fetch upcoming Google Calendar events across all connected accounts",
  "inputSchema": {
    "type": "object",
    "properties": {
      "days": { "type": "integer", "default": 7, "minimum": 1, "maximum": 30,
                "description": "How many days ahead to look" },
      "account_email": { "type": "string",
                         "description": "Specific Google account email (omit for all accounts)" }
    }
  }
}
```

#### `create_calendar_event`
```json
{
  "name": "create_calendar_event",
  "description": "Create a new event on a connected Google Calendar",
  "inputSchema": {
    "type": "object",
    "properties": {
      "account_email": { "type": "string", "description": "Google account to create event on" },
      "title": { "type": "string" },
      "start_datetime": { "type": "string", "description": "ISO 8601 datetime, e.g. 2026-03-15T14:00:00-05:00" },
      "end_datetime": { "type": "string", "description": "ISO 8601 datetime" },
      "description": { "type": "string" },
      "attendees": { "type": "array", "items": { "type": "string" },
                     "description": "List of attendee email addresses" }
    },
    "required": ["account_email", "title", "start_datetime", "end_datetime"]
  }
}
```

#### `get_tasks_bulk`
```json
{
  "name": "get_tasks_bulk",
  "description": "Fetch all task lists and all incomplete tasks for a Google account in a single call",
  "inputSchema": {
    "type": "object",
    "properties": {
      "account_email": { "type": "string", "description": "Google account (defaults to primary)" }
    }
  }
}
```

#### `search_memories`
```json
{
  "name": "search_memories",
  "description": "Semantic search over Izzie's stored memories (facts, notes, past observations)",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "description": "Natural language query" },
      "limit": { "type": "integer", "default": 10, "minimum": 1, "maximum": 50 }
    },
    "required": ["query"]
  }
}
```

#### `search_entities`
```json
{
  "name": "search_entities",
  "description": "Search the knowledge graph for people, organizations, and topics",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "description": "Name or description to search" },
      "entity_type": { "type": "string", "enum": ["person", "organization", "topic", "project"],
                       "description": "Filter by entity type (optional)" },
      "limit": { "type": "integer", "default": 10, "minimum": 1, "maximum": 50 }
    },
    "required": ["query"]
  }
}
```

#### `get_entity_relationships`
```json
{
  "name": "get_entity_relationships",
  "description": "Get all known relationships for a named entity (who they work with, etc.)",
  "inputSchema": {
    "type": "object",
    "properties": {
      "name": { "type": "string", "description": "Entity name to look up" }
    },
    "required": ["name"]
  }
}
```

#### `get_context`
```json
{
  "name": "get_context",
  "description": "Get full personal context about a person or topic: memories, entities, and upcoming calendar",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "description": "Person name, email, or topic to contextualize" }
    },
    "required": ["query"]
  }
}
```

#### `list_accounts`
```json
{
  "name": "list_accounts",
  "description": "List connected Google accounts and their capabilities (calendar, tasks, email)",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}
```

#### `schedule_event`
```json
{
  "name": "schedule_event",
  "description": "Schedule a future reminder or system event",
  "inputSchema": {
    "type": "object",
    "properties": {
      "event_type": { "type": "string", "enum": ["reminder", "email_sync", "calendar_refresh"],
                      "description": "Type of event to schedule" },
      "scheduled_at": { "type": "string", "description": "ISO 8601 datetime in the future" },
      "message": { "type": "string", "description": "Reminder message (for reminder events)" }
    },
    "required": ["event_type", "scheduled_at"]
  }
}
```

#### `list_open_loops`
```json
{
  "name": "list_open_loops",
  "description": "List pending follow-up items and open loops tracked by Izzie",
  "inputSchema": {
    "type": "object",
    "properties": {
      "limit": { "type": "integer", "default": 20 }
    }
  }
}
```

#### `get_preferences`
```json
{
  "name": "get_preferences",
  "description": "Get the user's Izzie preferences (proactive features, notification settings)",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}
```

---

## 5. Crate Structure: `crates/trusty-mcp/`

```
crates/trusty-mcp/
├── Cargo.toml
└── src/
    ├── main.rs          # CLI entry: --stdio flag, HTTP SSE, config loading
    ├── server.rs        # McpServer struct: tools/list and tools/call dispatch
    ├── protocol.rs      # JSON-RPC 2.0 types (Request, Response, Error, Notification)
    ├── transport/
    │   ├── mod.rs
    │   ├── stdio.rs     # Stdio line-delimited JSON-RPC loop
    │   └── sse.rs       # axum SSE endpoint: /mcp/sse and /mcp/message
    └── tools/
        ├── mod.rs       # Tool registry: name → schema + handler
        ├── calendar.rs  # get_calendar_events, create_calendar_event
        ├── contacts.rs  # search_contacts
        ├── tasks.rs     # get_tasks_bulk
        ├── memory.rs    # search_memories
        ├── entities.rs  # search_entities, get_entity_relationships, get_context
        ├── accounts.rs  # list_accounts
        ├── scheduler.rs # schedule_event
        └── loops.rs     # list_open_loops, get_preferences
```

### 5.1 `Cargo.toml`

```toml
[package]
name = "trusty-mcp"
version.workspace = true
edition.workspace = true

[[bin]]
name = "trusty-mcp"
path = "src/main.rs"

[dependencies]
# Workspace
tokio = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
axum = { workspace = true }
tower = { workspace = true }
tower-http = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
clap = { workspace = true }
chrono = { workspace = true }
reqwest = { workspace = true }
rusqlite = { workspace = true }

# Intra-workspace
trusty-store = { path = "../trusty-store" }
trusty-chat = { path = "../trusty-chat" }
trusty-core = { path = "../trusty-core" }
trusty-models = { path = "../trusty-models" }
```

Note: no new external MCP SDK is required. The MCP protocol (JSON-RPC 2.0 with specific method names) is simple enough to implement directly in ~200 lines. Adding an external SDK would introduce a new dependency that may not be stable.

---

## 6. Protocol Implementation

### 6.1 JSON-RPC 2.0 types (`protocol.rs`)

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}
```

### 6.2 MCP method dispatch (`server.rs`)

```rust
impl McpServer {
    pub async fn handle(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        match req.method.as_str() {
            "initialize"       => self.handle_initialize(req).await,
            "tools/list"       => self.handle_tools_list(req).await,
            "tools/call"       => self.handle_tools_call(req).await,
            "ping"             => self.handle_ping(req),
            _                  => JsonRpcResponse::method_not_found(req.id),
        }
    }
}
```

### 6.3 `initialize` handshake

The client sends `initialize` with `protocolVersion` and `clientInfo`. The server responds with:

```json
{
  "protocolVersion": "2024-11-05",
  "capabilities": { "tools": {} },
  "serverInfo": { "name": "trusty-izzie", "version": "0.1.7" }
}
```

### 6.4 `tools/call` execution

Each tool call maps `params.name` to a handler function. Handlers accept `serde_json::Value` (the `arguments` field) and return `Result<String>`. The server wraps the result in the MCP content envelope:

```json
{
  "content": [{ "type": "text", "text": "<tool output>" }],
  "isError": false
}
```

Errors use `"isError": true` with the error message as `text`. This matches the MCP spec and lets the calling LLM see what went wrong.

---

## 7. Tool Implementation Strategy

### 7.1 How to share logic with `trusty-chat`

Three options were considered:

**Option A: Instantiate `ChatEngine` directly**
Create a `ChatEngine` instance inside `McpServer`. This gives access to all 35 existing tool implementations immediately. The `ChatEngine` accepts `api_base`, `api_key`, `model`, and `SqliteStore` — the MCP server already needs these.

**Option B: Extract shared logic to `trusty-core`**
Move tool handlers to `trusty-core` and have both `trusty-chat` and `trusty-mcp` call them. This is the cleanest architecture but requires significant refactoring of existing code.

**Option C: HTTP proxy — call trusty-api endpoints**
Have `trusty-mcp` call `http://127.0.0.1:3457/chat` or other trusty-api endpoints. This introduces a runtime dependency on trusty-api being up and adds latency for every tool call.

**Decision: Option A for v1 (use `ChatEngine`), plan Option B for v2.**

The `ChatEngine::execute_tool` method already has the exact signature needed: `(&self, name: &ToolName, input: &serde_json::Value) -> Result<String>`. The MCP server can construct a `ChatEngine` with the same `SqliteStore` as the rest of the system, call `execute_tool`, and return the result as MCP content.

For tools not yet implemented in `ChatEngine` (like `SearchMemories`, `SearchEntities`) or tools that currently return `"Tool not yet implemented"`, the MCP server can implement them directly using `SqliteStore` and `LanceStore`.

### 7.2 Composite tool: `get_context`

This tool does not correspond to a single `ToolName`. It will be implemented directly in `tools/entities.rs`:

```rust
pub async fn tool_get_context(engine: &ChatEngine, query: &str) -> Result<String> {
    // 1. SearchMemories for query
    let memories = engine.execute_tool(&ToolName::SearchMemories,
        &serde_json::json!({"query": query, "limit": 5})).await?;
    // 2. SearchEntities for query
    let entities = engine.execute_tool(&ToolName::SearchEntities,
        &serde_json::json!({"query": query, "limit": 5})).await?;
    // 3. GetCalendarEvents mentioning query (best-effort)
    let calendar = engine.execute_tool(&ToolName::GetCalendarEvents,
        &serde_json::json!({"days": 14})).await?;
    // Compose into a single context block
    Ok(format!("## Context for \"{}\"\n\n### Memories\n{}\n\n### Known Entities\n{}\n\n### Upcoming Calendar\n{}",
        query, memories, entities, calendar))
}
```

---

## 8. Transport Implementations

### 8.1 stdio (`transport/stdio.rs`)

```rust
pub async fn run_stdio(server: Arc<McpServer>) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();
    let mut writer = tokio::io::BufWriter::new(stdout);

    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() { continue; }
        let req: JsonRpcRequest = serde_json::from_str(&line)?;
        let resp = server.handle(req).await;
        let bytes = serde_json::to_vec(&resp)?;
        writer.write_all(&bytes).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
    }
    Ok(())
}
```

All server logging uses `tracing` writing to stderr (never stdout), so the JSON-RPC stream on stdout remains clean.

### 8.2 HTTP SSE (`transport/sse.rs`)

The SSE transport follows the MCP spec: the client opens a `GET /mcp/sse` connection and receives a `endpoint` event with the URL for sending messages. Then it POSTs requests to `POST /mcp/message`.

```rust
// GET /mcp/sse  — establish SSE stream, send endpoint event
async fn sse_handler(
    State(state): State<McpAppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // create a per-connection channel
    // send: data: {"event": "endpoint", "data": "/mcp/message?session=<uuid>"}
    // hold stream open for server->client notifications
}

// POST /mcp/message?session=<uuid>  — receive a JSON-RPC request, reply inline
async fn message_handler(
    State(state): State<McpAppState>,
    Query(params): Query<SessionParams>,
    Json(req): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    let resp = state.server.handle(req).await;
    Json(resp)
}
```

For v1, server-initiated notifications (e.g., resource change events) are not needed. The SSE connection exists only to satisfy the MCP transport requirement; actual request/response goes through POST.

### 8.3 `main.rs` CLI

```rust
#[derive(Parser)]
struct Cli {
    /// Run as a stdio MCP server (for Claude Desktop)
    #[arg(long)]
    stdio: bool,
    /// HTTP port for SSE transport (default 3458)
    #[arg(long, default_value = "3458")]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = load_config(None).await?;
    let sqlite = Arc::new(SqliteStore::open(&sqlite_path)?);
    let engine = ChatEngine::new(...).with_sqlite(sqlite);
    let server = Arc::new(McpServer::new(engine));

    if cli.stdio {
        transport::stdio::run_stdio(server).await
    } else {
        transport::sse::run_http(server, cli.port).await
    }
}
```

---

## 9. Claude Desktop Configuration

### 9.1 stdio config (simplest)

```json
{
  "mcpServers": {
    "izzie": {
      "command": "/Users/masa/.cargo/bin/trusty-mcp",
      "args": ["--stdio"],
      "env": {
        "TRUSTY_PRIMARY_EMAIL": "user@gmail.com",
        "GOOGLE_CLIENT_ID": "...",
        "GOOGLE_CLIENT_SECRET": "..."
      }
    }
  }
}
```

Config file location on macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`

### 9.2 HTTP SSE config (for persistent server)

```json
{
  "mcpServers": {
    "izzie": {
      "url": "http://127.0.0.1:3458/mcp/sse"
    }
  }
}
```

### 9.3 `trusty mcp install-claude` command

A subcommand in `trusty-cli` that auto-generates the Claude Desktop config:

```
trusty mcp install-claude [--transport stdio|sse] [--port 3458]
```

Implementation:
1. Find `trusty-mcp` binary path (`which trusty-mcp` or relative to current binary)
2. Read `~/Library/Application Support/Claude/claude_desktop_config.json` (create if missing)
3. Merge in the `"izzie"` server entry
4. Write back
5. Print: "Restart Claude Desktop to activate the Izzie MCP server."

This command lives in `crates/trusty-cli/src/main.rs` as a new `mcp` subcommand.

### 9.4 Cursor config

Cursor uses `~/.cursor/mcp.json` or a project-local `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "izzie": {
      "command": "/Users/masa/.cargo/bin/trusty-mcp",
      "args": ["--stdio"]
    }
  }
}
```

---

## 10. Workspace Changes

### 10.1 `Cargo.toml` workspace members

Add:
```toml
members = [
    ...
    "crates/trusty-mcp",    # NEW
]
```

### 10.2 No new workspace dependencies required

All dependencies needed by `trusty-mcp` are already in `[workspace.dependencies]`.

---

## 11. Implementation Plan (Ordered Steps)

### Phase 1: Core protocol (estimated 1 day)

1. Create `crates/trusty-mcp/` directory structure
2. Write `Cargo.toml` with workspace dependencies + intra-workspace crates
3. Implement `protocol.rs`: `JsonRpcRequest`, `JsonRpcResponse`, `JsonRpcError`
4. Implement `server.rs`: `McpServer` struct, `handle()` dispatcher, `initialize`, `tools/list`, `ping`
5. Add `crates/trusty-mcp` to workspace `Cargo.toml`
6. Verify: `cargo check -p trusty-mcp`

### Phase 2: stdio transport + first 3 tools (estimated 1 day)

7. Implement `transport/stdio.rs`
8. Implement `main.rs` with `--stdio` flag and basic config loading
9. Wire up `get_calendar_events`, `list_accounts`, `get_preferences` (these delegate to `ChatEngine::execute_tool`)
10. Manual test: `echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' | trusty-mcp --stdio`
11. Test with Claude Desktop by adding to config and restarting

### Phase 3: Remaining read tools (estimated 1 day)

12. `search_contacts`, `get_tasks_bulk`, `list_open_loops` (ChatEngine delegation)
13. `schedule_event` (ChatEngine delegation)
14. `search_memories`, `search_entities`, `get_entity_relationships` — these currently return `"Tool not yet implemented"` from `ChatEngine`. Implement them directly using `LanceStore` + `GraphStore` in `tools/memory.rs` and `tools/entities.rs`
15. `get_context` composite tool

### Phase 4: HTTP SSE transport (estimated 1 day)

16. Implement `transport/sse.rs` with axum SSE + POST message endpoint
17. Add `--port` CLI arg, run HTTP mode by default
18. Test with Cursor via HTTP SSE config

### Phase 5: Install command + polish (estimated half day)

19. Add `trusty mcp install-claude` subcommand in `trusty-cli`
20. Add `trusty mcp install-cursor` subcommand
21. Add logging: `tracing` to stderr (stdio-safe), `tracing-subscriber` with TRUSTY_LOG_LEVEL env var
22. Add to `Makefile` target: `make mcp` to build and install
23. Update `README.md` and `DEVELOPMENT.md` with MCP setup instructions

---

## 12. Complexity Estimates

| Component | Complexity | Notes |
|-----------|-----------|-------|
| `protocol.rs` (JSON-RPC types) | Low | ~50 lines, pure serde |
| `server.rs` (dispatcher) | Low | ~100 lines |
| `transport/stdio.rs` | Low | ~60 lines, tokio I/O |
| `transport/sse.rs` | Medium | ~150 lines, axum SSE + session management |
| Tool delegation via `ChatEngine` | Low | ~10 lines per tool, mostly pass-through |
| `search_memories` implementation | Medium | Needs `LanceStore` wired up |
| `search_entities` implementation | Medium | Needs `LanceStore` + `GraphStore` |
| `get_context` composite | Low-Medium | Calls 3 existing tools, composes output |
| `install-claude` CLI subcommand | Low | JSON file read/merge/write |
| Main.rs + config loading | Low | Pattern exists in trusty-api |

**Total estimated effort: 4-5 engineering days for a production-ready v1.**

---

## 13. Security Considerations

1. **Local-only by default**: Both stdio (process spawn) and HTTP SSE (127.0.0.1) are local. No external network exposure.

2. **No new auth surface**: The MCP server reuses OAuth tokens already stored in SQLite by the main Izzie daemon. No new secrets are introduced.

3. **`ExecuteShellCommand` excluded**: This tool is intentionally not exposed via MCP. Shell execution by an external AI client is a significant attack surface.

4. **iMessage/WhatsApp excluded by default**: These require Full Disk Access. They can be added as opt-in tools with an explicit `--enable-message-search` flag.

5. **Input validation**: All tool inputs pass through the existing `ChatEngine` validation logic, which already handles malformed inputs gracefully (returning error strings rather than panicking).

6. **HTTP SSE binding**: The SSE server binds to `127.0.0.1` by default, never `0.0.0.0`. An explicit `--host` flag would be needed to expose externally (documented as dangerous).

---

## 14. Example MCP Session (stdio)

```
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","clientInfo":{"name":"claude-desktop","version":"1.0"}}}
← {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"trusty-izzie","version":"0.1.7"}}}

→ {"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
← {"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"get_calendar_events",...},{"name":"search_contacts",...},...13 tools total]}}

→ {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_calendar_events","arguments":{"days":7}}}
← {"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"Personal calendar (user@gmail.com):\n• 2026-03-12T10:00:00 — Team standup (4 attendees)\n• 2026-03-13T14:00:00 — 1:1 with Sarah @ Zoom"}],"isError":false}}

→ {"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_context","arguments":{"query":"Sarah Johnson"}}}
← {"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"## Context for \"Sarah Johnson\"\n\n### Memories\n- Met Sarah at DevConf 2025, works on distributed systems at Acme Corp\n- Mentioned she's hiring for senior engineers\n\n### Known Entities\n- Sarah Johnson (person): sarah.johnson@acme.com | Role: Engineering Manager at Acme Corp\n\n### Upcoming Calendar\n• 2026-03-13T14:00:00 — 1:1 with Sarah @ Zoom"}],"isError":false}}
```

---

## 15. File Save Confirmation

This design document is saved at:
`/Users/masa/Projects/trusty-izzie/docs/specs/10-mcp-service.md`

No ticketing context was provided in this request. The document is saved locally only.
