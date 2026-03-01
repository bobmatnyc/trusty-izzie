# Workspace Architecture

## Cargo Workspace Layout

```
trusty-izzie/
├── Cargo.toml              # workspace root
├── Cargo.lock
├── .env.example
├── config/
│   └── default.toml        # default runtime config
├── data/                   # default data directory (gitignored)
│   ├── lancedb/
│   ├── kuzu/
│   ├── tantivy/
│   └── trusty.db           # SQLite
├── crates/
│   ├── trusty-models/
│   ├── trusty-embeddings/
│   ├── trusty-store/
│   ├── trusty-extractor/
│   ├── trusty-email/
│   ├── trusty-memory/
│   ├── trusty-chat/
│   ├── trusty-core/
│   ├── trusty-daemon/
│   ├── trusty-api/
│   ├── trusty-cli/
│   └── trusty-tui/
└── docs/
    └── specs/
```

The workspace `Cargo.toml` defines shared dependency versions via `[workspace.dependencies]` to prevent version drift across crates.

---

## Crate Descriptions

### `trusty-models`

**Purpose:** Pure, portable data structures shared across all crates. Contains no business logic, no I/O, no async. Depends only on `serde`, `chrono`, and `uuid`. This is the lingua franca of the workspace — every crate imports it, so it must remain dependency-light.

**Public API surface:**
- `Entity` — core entity struct with all fields
- `EntityType` — enum: Person, Company, Project, Tool, Topic, Location, ActionItem
- `Relationship` — directed edge with type, confidence, evidence string
- `RelationshipType` — enum: WorksFor, WorksWith, WorksOn, ReportsTo, Leads, ExpertIn, LocatedIn, PartnersWith, RelatedTo
- `Memory` — memory struct with decay metadata
- `MemoryCategory` — enum with associated decay rates
- `ChatSession`, `ChatMessage` — session and message records
- `RawEmail`, `ParsedEmail` — email representations pre/post processing
- `EntityExtractionResult` — LLM output parsed struct
- `Config` — top-level runtime configuration struct
- `EntityFingerprint` — dedup hash record

**Key dependencies:** `serde`, `serde_json`, `chrono`, `uuid`

**Depends on:** nothing (no other workspace crates)

---

### `trusty-core`

**Purpose:** Shared infrastructure — error types, logging setup, config loading, and core traits that define the contracts between layers. Any crate that needs a common error type or a trait definition imports `trusty-core`, not a peer crate. Keeps the dependency graph acyclic.

**Public API surface:**
- `TrustyError` — unified error enum (wraps storage, LLM, IO, parse errors)
- `TrustyResult<T>` — type alias for `Result<T, TrustyError>`
- `trait Embeddable` — anything that can produce a `Vec<f32>` embedding
- `trait Searchable` — anything that supports hybrid search queries
- `trait StorageBackend` — CRUD abstraction over storage layers
- `AppConfig` — validated config loaded from TOML + env vars
- `setup_tracing()` — initializes `tracing-subscriber` with JSON or pretty format
- `constants` module — model names, default ports, threshold values

**Key dependencies:** `thiserror`, `anyhow`, `tracing`, `tracing-subscriber`, `config` (crate), `serde`

**Depends on:** `trusty-models`

---

### `trusty-embeddings`

**Purpose:** Manages all embedding and search functionality. Wraps `fastembed-rs` for local ONNX inference (all-MiniLM-L6-v2, 384 dimensions). Owns the tantivy BM25 index. Implements Reciprocal Rank Fusion (RRF) to merge BM25 and vector search results into a single ranked list.

**Public API surface:**
- `EmbeddingEngine` — wraps `fastembed::TextEmbedding`, thread-safe via `Arc<Mutex<...>>`
  - `fn embed(texts: &[&str]) -> TrustyResult<Vec<Vec<f32>>>`
  - `fn embed_one(text: &str) -> TrustyResult<Vec<f32>>`
- `BM25Index` — thin wrapper over a `tantivy::Index`
  - `fn add_document(doc: &IndexDocument) -> TrustyResult<()>`
  - `fn search(query: &str, limit: usize) -> TrustyResult<Vec<BM25Result>>`
  - `fn update_document(id: &str, doc: &IndexDocument) -> TrustyResult<()>`
  - `fn commit() -> TrustyResult<()>`
- `HybridSearchEngine` — combines `EmbeddingEngine` + `BM25Index` + LanceDB vector results
  - `fn search(query: &str, vector_results: Vec<VectorResult>, bm25_results: Vec<BM25Result>, alpha: f32, k: u32) -> Vec<FusedResult>`
- `IndexDocument` — {entity_id, content, entity_type, user_id}
- `FusedResult` — {id, score, bm25_rank, vector_rank, source}
- RRF formula: `score = alpha/(K+vector_rank) + (1-alpha)/(K+bm25_rank)`, K=60, alpha=0.7

**Key dependencies:** `fastembed`, `tantivy`, `tokio`

**Depends on:** `trusty-models`, `trusty-core`

---

### `trusty-store`

**Purpose:** All persistence. Owns the three storage backends — LanceDB, Kuzu, and SQLite — and exposes a unified `Store` struct that higher-level crates use. Handles schema creation/migration on startup, connection pooling, and serialization between Rust types and storage formats.

**Public API surface:**
- `Store` — top-level handle, constructed once and passed via `Arc<Store>`
  - `fn new(config: &StorageConfig) -> TrustyResult<Self>`
  - `fn entities(&self) -> &EntityStore`
  - `fn memories(&self) -> &MemoryStore`
  - `fn graph(&self) -> &GraphStore`
  - `fn sessions(&self) -> &SessionStore`
  - `fn config_kv(&self) -> &ConfigStore`
- `EntityStore` — LanceDB + tantivy operations for entities
  - `fn upsert(entity: &Entity) -> TrustyResult<()>`
  - `fn vector_search(query_vec: Vec<f32>, limit: usize) -> TrustyResult<Vec<ScoredEntity>>`
  - `fn get_by_id(id: &str) -> TrustyResult<Option<Entity>>`
  - `fn get_by_normalized_form(form: &str) -> TrustyResult<Option<Entity>>`
  - `fn list(filter: EntityFilter, page: Page) -> TrustyResult<Vec<Entity>>`
- `MemoryStore` — LanceDB operations for memories
  - `fn upsert(memory: &Memory) -> TrustyResult<()>`
  - `fn vector_search(query_vec: Vec<f32>, limit: usize) -> TrustyResult<Vec<ScoredMemory>>`
  - `fn apply_decay(now: DateTime<Utc>) -> TrustyResult<u64>`  ← bulk update
  - `fn list_above_threshold(min_strength: f32) -> TrustyResult<Vec<Memory>>`
- `GraphStore` — Kuzu operations for relationships
  - `fn upsert_node(entity: &Entity) -> TrustyResult<()>`
  - `fn upsert_edge(rel: &Relationship) -> TrustyResult<()>`
  - `fn get_neighbors(entity_id: &str, depth: u8) -> TrustyResult<Vec<GraphNode>>`
  - `fn query(cypher: &str) -> TrustyResult<Vec<serde_json::Value>>`
- `SessionStore` — SQLite for chat sessions and messages
- `ConfigStore` — SQLite key-value for runtime config

**Key dependencies:** `lancedb`, `kuzu`, `rusqlite`, `r2d2` (SQLite pool), `arrow-array`, `tokio`

**Depends on:** `trusty-models`, `trusty-core`, `trusty-embeddings`

---

### `trusty-extractor`

**Purpose:** The entity extraction pipeline. Takes a batch of `ParsedEmail` values, calls the LLM (mistral-small via OpenRouter) to extract entities and relationships, parses and validates the JSON response, applies confidence filtering, and returns clean `EntityExtractionResult` values ready for storage.

**Public API surface:**
- `Extractor` — stateless (takes config + HTTP client)
  - `fn extract_batch(emails: &[ParsedEmail]) -> TrustyResult<Vec<EntityExtractionResult>>`
  - `fn extract_one(email: &ParsedEmail) -> TrustyResult<Option<EntityExtractionResult>>`
  - `fn is_spam_or_newsletter(email: &ParsedEmail) -> TrustyResult<bool>`
- `ExtractionConfig` — model, confidence threshold, max relationships, prompt templates
- `EntityParser` — validates and cleans raw LLM JSON output
  - `fn parse(raw: &str) -> TrustyResult<Vec<RawEntity>>`
  - `fn validate(entity: &RawEntity) -> bool`
- `PersonHeaderExtractor` — extracts persons from email headers only (not body)
  - `fn extract_from_headers(email: &ParsedEmail) -> Vec<PersonCandidate>`
- `DeduplicationEngine` — checks fingerprints before returning results
  - `fn fingerprint(name: &str, entity_type: EntityType) -> String`
  - `fn is_duplicate(fp: &str, store: &Store) -> TrustyResult<bool>`
  - `fn normalize(name: &str) -> String`  ← lowercase, trim, collapse whitespace

**Key dependencies:** `reqwest`, `serde_json`, `regex`, `sha2` (for fingerprints)

**Depends on:** `trusty-models`, `trusty-core`, `trusty-store`

---

### `trusty-email`

**Purpose:** Gmail integration. Manages OAuth2 token lifecycle (storage in SQLite, refresh flow), implements the Gmail REST API client for SENT-only incremental sync using history IDs, and hands off raw email batches to the extractor. Also handles calendar sync for the next 7 days.

**Public API surface:**
- `GmailClient` — authenticated HTTP client
  - `fn new(token_store: &SessionStore, account_id: &str) -> TrustyResult<Self>`
  - `fn sync_sent(cursor: Option<HistoryId>, batch_size: u32) -> TrustyResult<SyncResult>`
  - `fn get_message(id: &str) -> TrustyResult<RawEmail>`
  - `fn refresh_token_if_needed() -> TrustyResult<()>`
- `OAuthFlow` — local redirect server on port 8080
  - `fn start_auth_flow(client_id: &str, client_secret: &str) -> TrustyResult<OAuthTokens>`
  - `fn exchange_code(code: &str) -> TrustyResult<OAuthTokens>`
- `CalendarClient`
  - `fn get_events_next_7_days() -> TrustyResult<Vec<CalendarEvent>>`
- `SyncResult` — {emails: Vec<RawEmail>, new_cursor: HistoryId, has_more: bool}
- `HistoryId` — newtype over `u64`

**Key dependencies:** `reqwest`, `oauth2`, `tokio`, `tiny_http` (local redirect server)

**Depends on:** `trusty-models`, `trusty-core`, `trusty-store`

---

### `trusty-memory`

**Purpose:** The memory subsystem. Stores, retrieves, and maintains the user's memory layer — distinct from entity facts, memories are episodic observations ("Alice prefers async communication"). Implements temporal decay, composite ranking, consolidation of near-duplicates, and context assembly for the chat engine.

**Public API surface:**
- `MemoryManager`
  - `fn save(memory: &Memory, store: &Store, engine: &EmbeddingEngine) -> TrustyResult<()>`
  - `fn retrieve_context(query: &str, limit: usize, store: &Store, engine: &EmbeddingEngine, search: &HybridSearchEngine) -> TrustyResult<Vec<RankedMemory>>`
  - `fn run_decay(store: &Store) -> TrustyResult<DecayReport>`
  - `fn consolidate(store: &Store, engine: &EmbeddingEngine) -> TrustyResult<ConsolidationReport>`
  - `fn refresh_access(memory_id: &str, store: &Store) -> TrustyResult<()>`
- `RankedMemory` — {memory, composite_score, strength, confidence, importance}
- `DecayReport` — {updated: u64, archived: u64}
- `ConsolidationReport` — {merged: u64, kept: u64}
- Decay formula: `strength = exp(-decay_rate * days * (1 - importance * 0.5))`
- Composite score: `score = strength*0.5 + confidence*0.3 + importance*0.2`

**Key dependencies:** (no external HTTP dependencies — pure computation + storage calls)

**Depends on:** `trusty-models`, `trusty-core`, `trusty-store`, `trusty-embeddings`

---

### `trusty-chat`

**Purpose:** The chat engine. Manages conversation sessions, assembles system prompts with retrieved context, executes the tool call loop (up to 5 iterations), streams LLM responses, parses structured JSON output, and persists new memories from each turn.

**Public API surface:**
- `ChatEngine`
  - `fn new_session(user_id: &str, store: &Store) -> TrustyResult<ChatSession>`
  - `fn send_message(session_id: &str, message: &str, store: &Store, opts: &ChatOptions) -> TrustyResult<impl Stream<Item = ChatEvent>>`
  - `fn compress_session(session_id: &str, store: &Store) -> TrustyResult<()>`
  - `fn list_sessions(user_id: &str, store: &Store) -> TrustyResult<Vec<ChatSession>>`
- `ChatEvent` — enum: Token(String), ToolCall(ToolCallEvent), ToolResult(ToolResultEvent), Done(ChatTurn)
- `ChatTurn` — {message, tool_calls_made, memories_saved, tokens_used}
- `ToolRegistry` — registers and dispatches tool calls
  - `fn register(tool: Box<dyn ChatTool>)`
  - `fn dispatch(call: &ToolCall, store: &Store) -> TrustyResult<serde_json::Value>`
- `trait ChatTool` — `fn name() -> &str`, `fn schema() -> serde_json::Value`, `fn execute(args: serde_json::Value, store: &Store) -> TrustyResult<serde_json::Value>`

**Key dependencies:** `reqwest`, `tokio-stream`, `serde_json`

**Depends on:** `trusty-models`, `trusty-core`, `trusty-store`, `trusty-embeddings`, `trusty-memory`

---

### `trusty-daemon`

**Purpose:** The long-running background process. Starts the Unix socket IPC server, runs the periodic email sync + entity extraction loop, applies memory decay on a schedule, and coordinates all background tasks. Acts as the runtime host for all other crates.

**Public API surface:**
- `Daemon`
  - `fn start(config: AppConfig) -> TrustyResult<()>`  ← blocks until signal
  - `fn trigger_sync(account_id: &str) -> TrustyResult<SyncReport>`
  - `fn status() -> DaemonStatus`
- `IpcServer` — Unix socket listener at `{data_dir}/trusty.sock`
  - `fn listen(socket_path: &Path, handler: Arc<RequestHandler>) -> TrustyResult<()>`
- `SyncScheduler` — tokio interval-based scheduler
  - Configurable interval (default 30 min)
  - Runs: email sync → extraction → tantivy commit → decay
- `DaemonStatus` — {last_sync, next_sync, emails_processed, entities_total, memories_total, is_syncing}

**Key dependencies:** `tokio`, `signal-hook`, `serde_json`

**Depends on:** all non-UI crates: `trusty-models`, `trusty-core`, `trusty-store`, `trusty-embeddings`, `trusty-extractor`, `trusty-email`, `trusty-memory`, `trusty-chat`

---

### `trusty-api`

**Purpose:** Axum-based HTTP server. Exposes REST endpoints for chat, entity browsing, memory listing, sync control, and OAuth flows. Supports SSE streaming for chat responses. Designed to be embedded inside `trusty-daemon` or run standalone.

**Public API surface:**
- `fn create_router(state: AppState) -> axum::Router`
- `AppState` — Arc-wrapped bundle of Store, ChatEngine, EmbeddingEngine, Config
- Routes (see `07-api-design.md` for full detail)

**Key dependencies:** `axum`, `tower`, `tower-http` (CORS, tracing), `tokio`

**Depends on:** `trusty-models`, `trusty-core`, `trusty-store`, `trusty-embeddings`, `trusty-chat`, `trusty-memory`

---

### `trusty-cli`

**Purpose:** Clap-based CLI binary. Sends commands to the running daemon via Unix socket IPC. Falls back to direct library calls if the daemon is not running. Outputs to stdout (plain text or JSON with `--json` flag).

**Public API surface:** (binary, no library API)
- Commands: `chat`, `entities list`, `entities get <id>`, `graph neighbors <id>`, `memories list`, `sync`, `status`, `auth google`

**Key dependencies:** `clap`, `serde_json`, `tokio`

**Depends on:** `trusty-models`, `trusty-core`; communicates with daemon via IPC

---

### `trusty-tui`

**Purpose:** Ratatui-based terminal UI. Full interactive experience: split-pane chat view, entity browser, memory viewer. Communicates with the daemon via Unix socket IPC for all data operations. Renders streaming tokens as they arrive.

**Public API surface:** (binary, no library API)
- Panels: Chat, Entities, Memories, Graph, Status

**Key dependencies:** `ratatui`, `crossterm`, `tokio`

**Depends on:** `trusty-models`, `trusty-core`; communicates with daemon via IPC

---

## Dependency Graph

```
                    trusty-models
                         │
                    trusty-core
                    ┌────┴─────────────────────┐
                    │                          │
           trusty-embeddings            (all other crates)
                    │
             trusty-store
             ┌──────┴──────────────────────────────────┐
             │              │               │           │
    trusty-extractor  trusty-email  trusty-memory  trusty-chat
             │              │               │           │
             └──────────────┴───────────────┴───────────┘
                                   │
                            trusty-daemon
                            ┌──────┴──────────────┐
                            │                     │
                       trusty-api         (IPC server)
                                          ┌───────┴───────┐
                                     trusty-cli       trusty-tui
```

Key rules enforced by this graph:
- `trusty-models` imports nothing from the workspace
- `trusty-core` imports only `trusty-models`
- `trusty-embeddings` imports only `trusty-models` + `trusty-core`
- `trusty-store` does not import `trusty-extractor`, `trusty-email`, `trusty-memory`, or `trusty-chat` (no circular deps)
- `trusty-cli` and `trusty-tui` do not import storage or engine crates directly — IPC only
- `trusty-daemon` is the only crate allowed to import everything

---

## Workspace `Cargo.toml` Structure

```toml
[workspace]
members = [
    "crates/trusty-models",
    "crates/trusty-core",
    "crates/trusty-embeddings",
    "crates/trusty-store",
    "crates/trusty-extractor",
    "crates/trusty-email",
    "crates/trusty-memory",
    "crates/trusty-chat",
    "crates/trusty-daemon",
    "crates/trusty-api",
    "crates/trusty-cli",
    "crates/trusty-tui",
]
resolver = "2"

[workspace.dependencies]
# Shared versions — all crates reference these
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
thiserror = "1"
anyhow = "1"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4", "serde"] }
reqwest = { version = "0.11", features = ["json", "stream"] }

# Storage
lancedb = "0.7"
rusqlite = { version = "0.31", features = ["bundled"] }
kuzu = "0.5"

# Search
tantivy = "0.21"
fastembed = "3"

# HTTP server
axum = { version = "0.7", features = ["macros"] }
tower = "0.4"
tower-http = { version = "0.5", features = ["cors", "trace"] }

# UI
ratatui = "0.26"
crossterm = "0.27"
clap = { version = "4", features = ["derive"] }

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.75"
```

---

## Feature Flags

| Crate             | Feature          | Effect                                    |
|-------------------|------------------|-------------------------------------------|
| `trusty-store`    | `s3`             | Enables LanceDB S3 backend                |
| `trusty-api`      | `tls`            | Enables rustls for HTTPS                  |
| `trusty-daemon`   | `full`           | Includes API + IPC (default)              |
| `trusty-embeddings` | `gpu`          | Enables ONNX GPU execution (future)       |

---

## Build Targets

```bash
# Run the daemon (primary binary)
cargo run -p trusty-daemon

# Run the TUI
cargo run -p trusty-tui

# Run the CLI
cargo run -p trusty-cli -- chat "Who does Alice work with?"

# Run the API standalone
cargo run -p trusty-api

# Run all tests
cargo test --workspace

# Build release binaries
cargo build --release --workspace
```
