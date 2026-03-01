# trusty-izzie: Project Overview

## Vision and Goals

trusty-izzie is a headless, local-first personal AI assistant written in Rust. It learns about the user's professional world by analyzing their sent email history, building a persistent knowledge graph of people, companies, and projects. All computation runs locally — no cloud databases, no per-query API costs beyond the LLM call itself.

The assistant exposes a chat interface that can be consumed via a TUI, a CLI, or an HTTP API. The headless core handles all logic: email ingestion, entity extraction, memory management, and LLM orchestration. UI layers are thin clients that communicate with the core over a Unix socket (IPC) or HTTP.

**Primary goals:**

1. Zero cloud database costs — all storage is local file-based (LanceDB, Kuzu, SQLite, tantivy)
2. Privacy by default — emails and knowledge graph never leave the machine
3. Minimal noise — strict confidence thresholds and extraction rules prevent garbage in the knowledge graph
4. On-prem deployable — run on a home server, Raspberry Pi, or developer laptop
5. Extensible UI — same daemon serves CLI power users and TUI explorers equally

---

## Key Differences from izzie2

| Dimension            | izzie2 (predecessor)                    | trusty-izzie                                      |
|----------------------|-----------------------------------------|---------------------------------------------------|
| Runtime language     | TypeScript / Node.js                    | Rust                                              |
| Hosting              | Vercel (serverless functions)           | Local daemon process                              |
| Vector DB            | Weaviate (cloud-hosted)                 | LanceDB (local file, optional S3)                 |
| Graph DB             | None                                    | Kuzu (embedded)                                   |
| Full-text search     | None                                    | tantivy (BM25)                                    |
| Embeddings           | OpenAI `text-embedding-ada-002`         | fastembed-rs (local ONNX, all-MiniLM-L6-v2)       |
| LLM provider         | OpenAI direct                           | OpenRouter (multi-model)                          |
| Cost per query       | $0.001–0.01 embedding + LLM             | $0 embedding + LLM only                           |
| Entity noise         | High (confidence ≥ 0.7, body mentions)  | Low (confidence ≥ 0.85, headers only for persons) |
| UI                   | Next.js web app                         | TUI + CLI + HTTP API                              |
| Auth                 | Clerk / NextAuth                        | Local single-user API key                         |
| Deployment           | Vercel + Railway                        | Single binary + cargo install                     |

---

## Tech Stack Summary

| Layer             | Technology              | Role                                                          |
|-------------------|-------------------------|---------------------------------------------------------------|
| Language          | Rust (stable)           | All application code                                          |
| Workspace         | Cargo workspace         | 12 crates with strict dependency boundaries                   |
| LLM routing       | OpenRouter API          | Chat (claude-sonnet-4.5), extraction (mistral-small)          |
| Vector storage    | LanceDB                 | Entity + memory embeddings (384-dim), optional S3 backend     |
| Graph DB          | Kuzu (embedded)         | Entity relationships (WORKS_FOR, WORKS_WITH, etc.)            |
| Relational        | SQLite via rusqlite     | Auth tokens, cursors, sessions, config, dedup fingerprints    |
| Full-text search  | tantivy                 | BM25 index over entity content                                |
| Embeddings        | fastembed-rs (ONNX)     | all-MiniLM-L6-v2, 384 dimensions, fully offline               |
| Email source      | Gmail REST API          | SENT label only, history-ID-based incremental sync            |
| HTTP server       | Axum                    | REST API + SSE streaming                                      |
| TUI               | Ratatui                 | Terminal user interface                                       |
| CLI               | Clap                    | Command-line interface                                        |
| Serialization     | serde + serde_json      | All data interchange                                          |
| Async runtime     | Tokio                   | Async I/O throughout                                          |
| Logging           | tracing + tracing-subscriber | Structured logging with spans                          |

---

## Local-First Design Principles

**1. No network required for core intelligence.**
Embeddings are computed locally via fastembed-rs using an ONNX model bundled at compile time or downloaded on first run. The knowledge graph and memory system operate entirely on-disk. Only LLM calls and Gmail sync require internet access.

**2. Data ownership is absolute.**
All user data — emails processed, entities extracted, memories stored — lives in a local directory (`~/.trusty-izzie/` by default). The user can inspect, back up, or delete this directory at any time.

**3. Cost is bounded and predictable.**
The only ongoing cost is LLM API usage (OpenRouter). There are no per-seat, per-query, or per-GB database fees.

**4. Storage is tiered and explainable.**
- LanceDB: vector embeddings and semantic search
- Kuzu: structured relationship queries ("who does Alice work with?")
- SQLite: operational state, sessions, cursors — human-readable with sqlite3 CLI
- tantivy: keyword search — index files on disk

**5. Single-user, zero multi-tenancy overhead.**
No user accounts, no row-level security, no tenant isolation. The system is designed for one person. `userId` in storage schemas is a single fixed value from config, used for future extensibility without re-engineering storage.

**6. The daemon is optional but recommended.**
All functionality is accessible via direct library calls. The daemon adds background sync and a persistent IPC server so that CLI and TUI commands respond instantly without re-initializing all storage backends.

---

## Data Flow Diagram

```
  Gmail API (SENT label)
         │
         ▼
  ┌─────────────────────┐
  │   trusty-email      │   OAuth2 refresh, history.list, message.get
  │   (sync client)     │   Batch 50 emails, cursor = historyId
  └────────┬────────────┘
           │  Vec<RawEmail>
           ▼
  ┌─────────────────────┐
  │  trusty-extractor   │   1. Spam filter (LLM, mistral-small)
  │  (entity pipeline)  │   2. Entity extract (LLM, mistral-small)
  │                     │   3. Parse + validate JSON
  │                     │   4. Confidence filter (≥ 0.85)
  │                     │   5. Dedup (normalized form + fingerprint)
  └──┬──────────────────┘
     │                  │
     │ Entities         │ Relationships
     ▼                  ▼
  ┌──────────┐    ┌────────────┐
  │ LanceDB  │    │   Kuzu     │    + tantivy BM25 index update
  │ (vectors)│    │  (graph)   │
  └──────────┘    └────────────┘
                        │
                        │ (background daemon loop, every 30 min)
  ─────────────────────────────────────────────────────────────────
                        │
                        │ (user initiates chat)
                        ▼
  ┌─────────────────────────────────────────────┐
  │              trusty-chat                    │
  │                                             │
  │  1. Assemble context (hybrid search)        │
  │     BM25 (tantivy) + vector (LanceDB)       │
  │     fused via RRF (alpha=0.7, K=60)         │
  │                                             │
  │  2. Build system prompt                     │
  │     identity + context + tool defs          │
  │                                             │
  │  3. LLM call (OpenRouter, claude-sonnet-4.5)│
  │     streaming response                      │
  │                                             │
  │  4. Parse structured response               │
  │     { message, memoriesToSave[] }           │
  │                                             │
  │  5. Persist memories → LanceDB              │
  │     Update session → SQLite                 │
  └──────────┬──────────────────────────────────┘
             │
     ┌───────┴────────┐
     │                │
     ▼                ▼
  trusty-api       trusty-tui / trusty-cli
  (Axum HTTP)      (Unix socket IPC)
  SSE stream       direct print / TUI widget
```

---

## Non-Goals

- Multi-user support (single user only)
- Mobile or web UI (use the HTTP API if needed)
- Ingesting emails from providers other than Gmail (v1)
- Fine-tuning or local LLM hosting (OpenRouter handles routing)
- Real-time calendar push notifications (polling only, v1)
