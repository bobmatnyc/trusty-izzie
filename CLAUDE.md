# trusty-izzie — Project Instructions for Claude

## What This Project Is

trusty-izzie is a **headless Rust application** — a local-first personal AI assistant that learns from the user's outgoing email and provides a conversational interface to their professional context. It has no cloud database, no managed vector store, and no hosting costs. All data lives on the local machine.

**Architecture pattern**: Follows `ai-commander` — a Rust cargo workspace with a background daemon, Unix socket IPC, and multiple UI implementations (CLI, TUI, REST API) all talking to a shared headless core.

**Current status**: Scaffolded. All 12 crates exist with stub implementations. The workspace compiles clean (`cargo check` passes). Data migration from Weaviate is complete — the local stores at `~/.local/share/trusty-izzie/` are populated.

---

## Tech Stack

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust (edition 2021) | Tokio async runtime |
| Vector DB | LanceDB (`lancedb` crate) | Local `.lance` files, S3-optional |
| Graph DB | Kuzu (`kuzu` crate 0.9) | Embedded, no server required |
| Relational | SQLite (`rusqlite` bundled) | Auth tokens, cursors, sessions |
| Full-text | tantivy 0.22 | BM25 index, pure Rust |
| Embeddings | fastembed 4 | Local ONNX, all-MiniLM-L6-v2 (384-dim), MPS on Apple Silicon |
| LLM | OpenRouter | Chat: claude-sonnet-4-5, Extraction: mistral-small-3.1-24b |
| HTTP client | reqwest 0.12 | Gmail API, OpenRouter API |
| HTTP server | axum 0.7 | REST API + SSE streaming |
| CLI | clap 4 (derive) | `trusty` binary |
| TUI | ratatui 0.29 + crossterm 0.28 | Terminal UI |
| Email | Gmail REST API | OAuth2 PKCE, SENT folder only |
| Tunneling | ngrok | Domain: `izzie.ngrok.dev` → port 3456 |

---

## Build & Run

```bash
# Check everything compiles (fast)
cargo check

# Build all crates (dev)
cargo build

# Build release binaries
cargo build --release

# Run a specific binary
cargo run --bin trusty         # CLI
cargo run --bin trusty-daemon  # Daemon
cargo run --bin trusty-api     # REST API server

# Run tests (when implemented)
cargo test

# Check a single crate
cargo check -p trusty-core
cargo check -p trusty-store
```

**Required env vars** (copy `.env.example` → `.env`):
```bash
OPENROUTER_API_KEY=sk-or-v1-...
GOOGLE_CLIENT_ID=...
GOOGLE_CLIENT_SECRET=...
TRUSTY_PRIMARY_EMAIL=bobmatnyc@gmail.com
TRUSTY_DATA_DIR=~/.local/share/trusty-izzie
```

**Data is already migrated** — `~/.local/share/trusty-izzie/` contains:
- `lance/entities.lance` — 6,774 entities (384-dim embeddings)
- `lance/memories.lance` — 29 memories
- `kuzu/` — 1,338 graph nodes, 28 relationship edges (42MB)
- `instance.json` — instance ID `42a923e9bd673e38` (SHA256 of `bobmatnyc@gmail.com`)

**ngrok tunnel**: `ngrok start izzie` → `https://izzie.ngrok.dev` → `localhost:3456`

---

## Workspace Structure

```
trusty-izzie/
├── Cargo.toml              # Workspace root, all shared deps
├── config/default.toml     # Default configuration
├── .env                    # Secrets (gitignored)
├── .env.example            # Template (tracked)
├── migration/              # One-time migration scripts (keep, not run again)
│   └── migrate_from_weaviate.py
├── docs/specs/             # 8 comprehensive spec documents
└── crates/
    ├── trusty-models/      # Pure data types — FOUNDATION (no deps on other crates)
    ├── trusty-embeddings/  # fastembed + tantivy BM25 + RRF hybrid search
    ├── trusty-store/       # LanceDB + Kuzu + SQLite persistence
    ├── trusty-extractor/   # LLM entity extraction pipeline
    ├── trusty-email/       # Gmail OAuth2 + incremental sync
    ├── trusty-memory/      # Temporal decay + hybrid context retrieval
    ├── trusty-chat/        # Chat engine: session, tools, streaming
    ├── trusty-core/        # Shared config, TrustyError, logging
    ├── trusty-daemon/      # Background daemon + IPC server (binary)
    ├── trusty-api/         # Axum REST API + SSE (binary)
    ├── trusty-cli/         # Clap CLI (binary)
    └── trusty-tui/         # Ratatui TUI (binary)
```

**Dependency order** (implement bottom-up):
```
trusty-models
  └── trusty-embeddings
        └── trusty-store
              ├── trusty-extractor
              ├── trusty-email
              └── trusty-memory
                    └── trusty-chat
                          └── trusty-core
                                ├── trusty-daemon (binary)
                                ├── trusty-api    (binary)
                                ├── trusty-cli    (binary)
                                └── trusty-tui    (binary)
```

---

## Key Design Decisions

### Single-Tenant Architecture
Each trusty-izzie instance serves exactly one user. No multi-tenancy.
- Instance ID = `SHA256(primary_email)[:16]` = `42a923e9bd673e38`
- LanceDB tables: `entities`, `memories` (no user prefix)
- Kuzu nodes: `Person`, `Company`, `Project`, etc. (no scoping needed)
- SQLite: single user's config, auth tokens, sessions

### Local-First Storage
No cloud databases. Everything in `~/.local/share/trusty-izzie/`:
- LanceDB stores vectors as Apache Arrow files locally (or to S3 if configured)
- Kuzu is embedded — no kuzu server process
- tantivy index is a local directory
- SQLite is a single file

### Hybrid Search (BM25 + Vector + RRF)
- tantivy handles BM25 keyword search
- fastembed generates 384-dim vectors locally (MPS on Apple Silicon)
- RRF fusion: `score = 0.7/(60+vector_rank) + 0.3/(60+bm25_rank)`
- Default: hybrid mode, alpha=0.7 (70% vector, 30% BM25)

### Entity Discovery Philosophy
Stricter than izzie2:
- **Persons from email headers only** (From/To/CC) — never body
- **Confidence ≥ 0.85** required (migration found 266 izzie2 entities below this)
- **Min 2 occurrences** before storing (prevents one-off noise)
- **Max 3 relationships per email** (izzie2 had 5 — too noisy)
- **SENT emails only** — outgoing mail reflects real relationships

### API-First + IPC Pattern
Follows `ai-commander`:
- Daemon owns all state and runs continuously
- CLI/TUI communicate via Unix socket IPC (`/tmp/trusty-izzie.sock`)
- REST API is a separate server (axum) for web UI / external integration
- All three UIs call the same business logic via shared crates

### Kuzu RELATED_TO Schema Issue (Known)
Current DDL defines `RELATED_TO` as `FROM Topic TO Topic`. The Weaviate migration found 6 relationships where `RELATED_TO` connects non-Topic entities (Person→Topic, Company→Topic). Fix: widen the RELATED_TO rel table or add specific typed edges.

---

## Spec Documents

All detailed implementation specs are in `docs/specs/`:

| File | Contents |
|------|----------|
| `00-overview.md` | Vision, goals, non-goals, data flow |
| `01-workspace-architecture.md` | All 12 crates, public APIs, dependency rules |
| `02-entity-discovery.md` | Full extraction pipeline + LLM prompt |
| `03-memory-system.md` | Decay formula, retrieval pipeline, LanceDB schema |
| `04-chat-engine.md` | Session management, tool loop, system prompt |
| `05-storage-layer.md` | LanceDB Arrow schemas, Kuzu DDL, SQLite tables |
| `06-email-pipeline.md` | Gmail OAuth2 PKCE, incremental sync, error handling |
| `07-api-design.md` | REST endpoints, SSE streaming, IPC protocol |

**Read the relevant spec before implementing any crate.**

---

## Implementation Status

All crates are scaffolded with `todo!()` stubs. Start from the bottom of the dependency chain.

| Crate | Status | Priority |
|-------|--------|----------|
| `trusty-models` | ✅ Scaffolded (full types defined) | — |
| `trusty-embeddings` | 🔲 Stub | 1st |
| `trusty-store` | 🔲 Stub | 2nd |
| `trusty-extractor` | 🔲 Stub | 3rd |
| `trusty-email` | 🔲 Stub | 4th |
| `trusty-memory` | 🔲 Stub | 4th |
| `trusty-chat` | 🔲 Stub | 5th |
| `trusty-core` | ✅ Config + errors scaffolded | — |
| `trusty-daemon` | 🔲 Stub | 6th |
| `trusty-api` | 🔲 Stub | 7th |
| `trusty-cli` | ✅ Full Clap command tree | 7th (wire up) |
| `trusty-tui` | 🔲 Basic layout stub | Last |

---

## Development Workflow

### Adding a new feature
1. Read the relevant spec in `docs/specs/`
2. Implement the lowest crate in the chain first
3. Write unit tests alongside implementation (in `#[cfg(test)]` modules)
4. Wire up in the CLI (`trusty-cli/src/main.rs`)
5. Add REST endpoint if needed (`trusty-api/src/handlers/`)

### Error handling conventions
- Library crates return `TrustyError` (from `trusty-core::error`)
- Binary crates use `anyhow::Result` for main functions
- Never use `.unwrap()` or `.expect()` in library crates
- Use `thiserror` for typed errors, `anyhow` for context chains

### Async conventions
- All I/O is async (tokio)
- Use `Arc<Store>` for shared database access across tasks
- Blocking operations (Kuzu, tantivy) go in `tokio::task::spawn_blocking`

### Testing
- Unit tests: `#[cfg(test)]` at bottom of each module
- Integration tests: `crates/{crate}/tests/` directory
- Prefer real SQLite in-memory (`:memory:`) for store tests over mocking
- Use `tempfile::TempDir` for LanceDB and Kuzu test directories

---

## Google OAuth Setup

The app uses existing izzie2 Google OAuth credentials:
- Client ID: `409456389838-...`
- Redirect URI (local): `http://localhost:8080/callback`
- Redirect URI (ngrok): `https://izzie.ngrok.dev/api/auth/google/callback`

**To add the ngrok redirect URI**: Google Cloud Console → APIs & Services → Credentials → OAuth 2.0 Client → Add `https://izzie.ngrok.dev/api/auth/google/callback` to Authorized redirect URIs.

---

## Migration Notes

- Migration from Weaviate ran successfully on 2026-03-01
- 6,774 entities and 29 memories imported, re-embedded at 384-dim
- Full migration report: `~/.local/share/trusty-izzie/instance.json`
- Migration issues log: `migration/migration.log` (local, gitignored)
- Do not re-run `migrate_from_weaviate.py` — it will overwrite the local stores
