# PM Agent Memories — trusty-izzie
# Initialized: 2026-03-01

## Project Identity

- Project: trusty-izzie — headless Rust personal AI assistant
- Location: /Users/masa/Projects/trusty-izzie
- Primary user: bobmatnyc@gmail.com (instance ID: 42a923e9bd673e38)
- GitHub repo: bobmatnyc/trusty-izzie (private, not yet pushed)
- Public URL: https://izzie.ngrok.dev → localhost:3456 (ngrok tunnel: `ngrok start izzie`)

## Architecture Decisions (Critical)

- Rust cargo workspace, 12 crates, follows ai-commander pattern
- Single-tenant: one instance per user, no multi-tenancy
- Instance ID derived from SHA256(primary_email)[:16]
- LanceDB tables named simply "entities" and "memories" (no user prefix)
- All data at ~/.local/share/trusty-izzie/ (gitignored)
- fastembed local ONNX inference (all-MiniLM-L6-v2, 384-dim, MPS on Apple Silicon)
- Hybrid search: tantivy BM25 + fastembed vector + RRF (K=60, alpha=0.7)
- Graph DB: Kuzu embedded (v0.9), no server required
- Gmail: SENT folder only, confidence ≥ 0.85, persons from headers only

## Current Status

- Scaffolded: all 12 crates compile clean, all stubs with todo!()
- Data migrated: 6,774 entities + 29 memories from Weaviate → local LanceDB + Kuzu
- Known issue: Kuzu RELATED_TO DDL too restrictive (Topic→Topic only), needs ANY→ANY
- Next step: implement trusty-embeddings (fastembed + tantivy), then trusty-store
- GitHub repo not yet created (pending git access)

## Tech Stack Summary

- Language: Rust 2021 edition, tokio async runtime
- Storage: LanceDB + Kuzu + SQLite (rusqlite bundled) + tantivy
- LLM: OpenRouter — chat: claude-sonnet-4-5, extraction: mistral-small-3.1-24b
- HTTP: axum 0.7 (server), reqwest 0.12 (client)
- CLI: clap 4 (derive macros), `trusty` binary
- TUI: ratatui 0.29 + crossterm 0.28
- OAuth: PKCE flow, local redirect server on port 8080

## Credentials & Secrets

- Google OAuth: same client ID/secret as izzie2
- OpenRouter: same API key as izzie2
- ngrok: domain izzie.ngrok.dev reserved, tunnel config in ~/.config/ngrok/ngrok.yml
- All secrets in .env (gitignored), template in .env.example
- Google OAuth redirect URIs: need to add https://izzie.ngrok.dev/api/auth/google/callback to Google Console

## Development Workflow Preferences

- Implement crates bottom-up: models → embeddings → store → extractor/email/memory → chat → core → binaries
- Read docs/specs/ before implementing any crate
- Use rust-engineer agent for all implementation
- Use cargo check frequently (fast feedback)
- Blocking ops (Kuzu, tantivy writes) go in tokio::task::spawn_blocking
- Unit tests in #[cfg(test)] modules, integration tests in crates/{name}/tests/

## Migration Notes (Do Not Re-Run)

- Migration from Weaviate ran 2026-03-01, populated local stores
- Script: migration/migrate_from_weaviate.py (keep, do not delete)
- Issues found: 109 persons from body (not headers) in izzie2, 266 below 0.85 threshold
- Weaviate tenant: W1SkmfubAgAw1WzkmebBPJDouzuFoaCV

## PR / Git Workflow

- Main branch: main
- Feature branches for all changes
- PRs merge to main when bobmatnyc@users.noreply.github.com is git user
- GitHub repo: bobmatnyc/trusty-izzie (private, create when git access available)
