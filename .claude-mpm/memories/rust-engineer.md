# Rust Engineer Memories — trusty-izzie
# Initialized: 2026-03-01

## Project Context

Rust cargo workspace (edition 2021), 12 crates, tokio async runtime.
All crates currently scaffolded with todo!() stubs — cargo check passes.
Implement bottom-up: models → embeddings → store → extractor/email/memory → chat → binaries.

## Crate Dependency Order (implement in this sequence)

1. trusty-models (done — full types defined)
2. trusty-embeddings (fastembed + tantivy BM25 + RRF)
3. trusty-store (LanceDB + Kuzu + SQLite)
4. trusty-extractor + trusty-email + trusty-memory (parallel)
5. trusty-chat
6. trusty-core (config loading already stubbed)
7. trusty-daemon, trusty-api, trusty-cli (wire up), trusty-tui

## Key Implementation Patterns

- All library crates return TrustyError (from trusty-core::error, thiserror)
- Binary crates use anyhow::Result for main()
- Never .unwrap() or .expect() in library code
- Kuzu queries are synchronous → wrap in tokio::task::spawn_blocking
- tantivy IndexWriter is also synchronous → spawn_blocking
- LanceDB is async-native (await directly)
- fastembed model load is blocking → spawn_blocking, then cache in Arc<Embedder>

## Storage Conventions

- LanceDB tables: "entities" (6,774 rows), "memories" (29 rows) — single-tenant, no prefix
- Vector dim: 384 (all-MiniLM-L6-v2)
- Kuzu node types: Person, Company, Project, Tool, Topic, Location, ActionItem
- Kuzu edge types: WORKS_FOR, WORKS_WITH, WORKS_ON, REPORTS_TO, LEADS, EXPERT_IN, LOCATED_IN, PARTNERS_WITH, RELATED_TO
- KNOWN ISSUE: RELATED_TO DDL is Topic→Topic only; fix to accept any node type pair
- SQLite tables: oauth_tokens, email_cursors, chat_sessions, chat_messages, entity_fingerprints, config

## Hybrid Search Implementation

```rust
// RRF fusion formula
fn rrf_score(vector_rank: Option<usize>, bm25_rank: Option<usize>, alpha: f32) -> f32 {
    const K: f32 = 60.0;
    let v = vector_rank.map_or(0.0, |r| alpha / (K + r as f32));
    let b = bm25_rank.map_or(0.0, |r| (1.0 - alpha) / (K + r as f32));
    v + b
}
// Default: alpha=0.7 (70% vector, 30% BM25)
```

## Entity Extraction Rules (STRICT)

- Person entities: headers only (From/To/CC), never body
- Confidence threshold: ≥ 0.85
- Min occurrences: 2 before persisting (tracked in SQLite entity_fingerprints)
- Max relationships per email: 3
- SENT folder only — never process received emails

## LLM Configuration

- Chat model: "anthropic/claude-sonnet-4-5" (via OpenRouter)
- Extraction model: "mistralai/mistral-small-3.1-24b-instruct" (cheaper, fast)
- Base URL: "https://openrouter.ai/api/v1"
- Max tool iterations per chat turn: 5
- Context: inject up to 10 memories + 15 entities per message

## Memory Decay Formula

```rust
fn decay_strength(importance: f32, decay_rate: f32, days_since_access: f32) -> f32 {
    let effective_rate = decay_rate * (1.0 - importance * 0.5);
    (-effective_rate * days_since_access).exp()
}

fn ranking_score(strength: f32, confidence: f32, importance: f32) -> f32 {
    strength * 0.5 + confidence * 0.3 + importance * 0.2
}
```

## Cargo Workspace Deps (use workspace = true)

Key shared deps: tokio, serde, serde_json, reqwest, axum, clap, ratatui, crossterm,
anyhow, thiserror, tracing, tracing-subscriber, chrono, uuid, rusqlite,
lancedb, arrow, arrow-array, arrow-schema, kuzu, fastembed, tantivy,
oauth2, config, dotenvy, futures, async-trait, once_cell, regex, interprocess

## Error Handling Pattern

```rust
// In library crates:
use trusty_core::error::{TrustyError, Result};

impl MyStruct {
    pub async fn do_thing(&self) -> Result<Output> {
        let data = self.store.get(id).await
            .map_err(|e| TrustyError::Storage(e.to_string()))?;
        Ok(data)
    }
}

// In binary crates (main.rs):
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // anyhow for rich error context
}
```

## Test Patterns

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_store_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).await.unwrap();
        // use real SQLite :memory: or temp dir — never mock the storage layer
    }
}
```
