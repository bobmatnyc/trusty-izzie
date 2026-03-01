# Development Guide

## Prerequisites

- Rust stable (1.75+): `rustup update stable`
- Python 3.14+ (Homebrew): for migration scripts only
- ngrok: `brew install ngrok` (already configured)
- Google Cloud Console access: to add OAuth redirect URIs

## First-Time Setup

```bash
cd /Users/masa/Projects/trusty-izzie

# 1. Copy env template
cp .env.example .env
# Edit .env — OPENROUTER_API_KEY, GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET are pre-filled

# 2. Build the workspace
cargo build

# 3. (Optional) Start ngrok tunnel for remote access
ngrok start izzie
# Exposes https://izzie.ngrok.dev → localhost:3456

# 4. Add Google OAuth redirect URI (one-time)
# Google Console → APIs & Services → Credentials → OAuth 2.0 Client:
# Add: https://izzie.ngrok.dev/api/auth/google/callback
```

## Data is Pre-Populated

The local stores are already populated from the Weaviate migration (run 2026-03-01):

```
~/.local/share/trusty-izzie/
├── instance.json          # Instance identity (instance_id, email)
├── lance/
│   ├── entities.lance/    # 6,774 entities (384-dim vectors)
│   └── memories.lance/    # 29 memories
└── kuzu/                  # 1,338 graph nodes, 28 edges (42MB)
```

**Do not re-run the migration script** — it will overwrite these stores.

## Build Commands

```bash
cargo check                          # Fast syntax/type check
cargo check -p trusty-store         # Check single crate
cargo build                          # Dev build (unoptimized)
cargo build --release                # Release build (strip + LTO)
cargo test                           # Run all tests
cargo test -p trusty-embeddings      # Test single crate
cargo run --bin trusty               # Run CLI
cargo run --bin trusty -- chat       # Run CLI with subcommand
cargo run --bin trusty-daemon        # Run daemon (foreground)
cargo run --bin trusty-api           # Run API server
```

## Implementation Order

Work bottom-up through the dependency chain. Each crate has a spec in `docs/specs/`.

### Phase 1 — Core Infrastructure

**trusty-embeddings** (`docs/specs/01-workspace-architecture.md`, `05-storage-layer.md`)
- `src/embedder.rs`: Wrap `fastembed::TextEmbedding` with `all-MiniLM-L6-v2`
  - Load model in `Arc<OnceCell<TextEmbedding>>`, cache after first use
  - `embed(&str) -> Result<Vec<f32>>` and `embed_batch(&[&str]) -> Result<Vec<Vec<f32>>>`
  - Apple Silicon: fastembed auto-detects MPS
- `src/bm25.rs`: Wrap `tantivy` with a simple `content` field schema
  - `index(id: &str, content: &str, entity_type: &str)` — add to index
  - `search(query: &str, limit: usize) -> Vec<Bm25Result>` — BM25 query
  - Commit index in `spawn_blocking`
- `src/hybrid.rs`: RRF fusion over both results
  - `alpha=0.7, K=60` — see CLAUDE.md formula

**trusty-store** (`docs/specs/05-storage-layer.md`)
- `src/lance.rs`: LanceDB client wrapping `lancedb::connect()`
  - Open existing `entities` and `memories` tables (already have data)
  - `add_entity(entity: &Entity, vector: Vec<f32>)` — insert Arrow record batch
  - `search_entities(vector: &[f32], limit: usize) -> Vec<EntityRow>` — ANN search
- `src/sqlite.rs`: rusqlite in WAL mode
  - `run_migrations()` — create tables if not exist
  - `get_token / set_token` — OAuth token CRUD
  - `get_cursor / set_cursor` — Gmail history ID cursor
- `src/graph.rs`: Kuzu node + edge operations
  - All queries via `spawn_blocking` (Kuzu is synchronous)
  - Fix RELATED_TO DDL: define as a generic rel table accepting any label pair

### Phase 2 — Business Logic

**trusty-extractor** (`docs/specs/02-entity-discovery.md`)
- OpenRouter HTTP call with the extraction prompt in `src/prompt.rs`
- Parse JSON response, validate confidence ≥ 0.85
- Check occurrence count in SQLite fingerprints table before persisting

**trusty-email** (`docs/specs/06-email-pipeline.md`)
- PKCE OAuth flow with tiny_http local redirect server on port 8080
- Gmail history.list API for incremental SENT sync
- `GmailClient::sync_sent(cursor: Option<HistoryId>, batch: u32) -> SyncResult`

**trusty-memory** (`docs/specs/03-memory-system.md`)
- `MemoryStore::save(memory: Memory, vector: Vec<f32>)` — write to LanceDB
- `MemoryRecaller::recall(query: &str, limit: usize) -> Vec<RankedMemory>`
  - Hybrid search → decay scoring → rank → refresh top-5 access timestamps

### Phase 3 — Chat Engine

**trusty-chat** (`docs/specs/04-chat-engine.md`)
- `ChatEngine::chat(session_id, message) -> impl Stream<Item=ChatChunk>`
- System prompt assembly (identity + context + cost awareness)
- Tool call loop (max 5 iterations)
- `SessionManager` — sliding window compression in SQLite

### Phase 4 — Interfaces

**trusty-daemon**: Wire `GmailClient` + `EntityExtractor` + `MemoryStore` into async polling loop
**trusty-api**: Implement axum handlers (currently all `todo!()`)
**trusty-cli**: Wire clap commands to `ChatEngine` / `Store` via IPC or direct call
**trusty-tui**: Build ratatui event loop over the basic layout stub

## Known Issues to Fix First

1. **Kuzu RELATED_TO schema** — current DDL is `FROM Topic TO Topic` only.
   Fix in `trusty-store/src/graph.rs` schema migration to accept any label pair:
   ```sql
   -- Replace:
   CREATE REL TABLE IF NOT EXISTS RELATED_TO(FROM Topic TO Topic, ...)
   -- With a separate junction table approach or generic edges
   ```

2. **fastembed v4 API** — the scaffold uses `todo!()` for fastembed; the actual v4 API is:
   ```rust
   use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
   let model = TextEmbedding::try_new(
       InitOptions::new(EmbeddingModel::AllMiniLML6V2)
   )?;
   let embeddings = model.embed(vec!["text"], None)?;  // -> Vec<Vec<f32>>
   ```

3. **Kuzu v0.9 query API** — uses `Connection::execute(query, params)` where params is
   a `HashMap<&str, &dyn KuzuValue>`. The scaffold has correct pattern.

## Cargo Conventions

```rust
// In each crate's Cargo.toml — use workspace versions:
[dependencies]
tokio = { workspace = true }
serde = { workspace = true }
# NOT:
tokio = "1"  # ❌ always use workspace = true
```

```rust
// Module structure for library crates:
// src/lib.rs — pub mod declarations + pub use re-exports
// src/error.rs — module-specific error variants (or reuse TrustyError)
// src/foo.rs  — implementation
```

## Useful Debug Commands

```bash
# Watch for compile errors as you type
cargo watch -x check

# Run with tracing logs
TRUSTY_LOG_LEVEL=debug cargo run --bin trusty

# Check LanceDB table contents (Python)
/opt/homebrew/bin/python3 -c "
import lancedb
db = lancedb.connect('$HOME/.local/share/trusty-izzie/lance')
t = db.open_table('entities')
print(t.count_rows())
print(t.to_pandas().head())
"

# Check Kuzu graph
/opt/homebrew/bin/python3 -c "
import kuzu
db = kuzu.Database('$HOME/.local/share/trusty-izzie/kuzu')
conn = kuzu.Connection(db)
r = conn.execute('MATCH (n:Person) RETURN n.value LIMIT 5')
while r.has_next(): print(r.get_next())
"
```

## Ngrok Tunnel

```bash
# Start the izzie tunnel (izzie.ngrok.dev → localhost:3456)
ngrok start izzie

# Or start all tunnels
ngrok start --all

# Check tunnel status
ngrok api tunnels list
```
