# Development Guide

## Prerequisites

- **Rust 1.75+** — install via [rustup](https://rustup.rs/): `rustup update stable`
- **macOS** (recommended) — Linux is experimental; Windows is not supported
- **Google Cloud account** — to create OAuth 2.0 credentials for Gmail/Calendar access
- **OpenRouter account** — for LLM inference (chat + entity extraction)
- **ngrok** (optional) — to expose a local port publicly for Telegram webhooks or OAuth callbacks
- **Telegram bot** (optional) — required only for the Telegram interface

## Clone and Build

```bash
git clone https://github.com/your-org/trusty-izzie.git
cd trusty-izzie

# Build the full workspace (dev build)
cargo build

# Or build release binaries
cargo build --release
```

Build artifacts are placed in `target/debug/` or `target/release/`.

## Environment Setup

```bash
# Copy the example file
cp .env.example .env

# Open and fill in required values
$EDITOR .env
```

At minimum, set:
- `OPENROUTER_API_KEY` — from [openrouter.ai/keys](https://openrouter.ai/keys)
- `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` — see **Google OAuth Setup** in `README.md`
- `TRUSTY_PRIMARY_EMAIL` — your Gmail address

See `.env.example` for full variable documentation.

## First-Time Setup

```bash
# 1. Authenticate with Google (opens browser)
cargo run --bin trusty -- auth google

# 2. Start the background daemon (foreground for debugging)
cargo run --bin trusty-daemon -- start --foreground

# 3. Trigger an initial email sync
cargo run --bin trusty -- sync --force

# 4. Start chatting
cargo run --bin trusty -- chat
```

On first run, the daemon creates `~/.local/share/trusty-izzie/` and generates a persistent
`instance.json` with a random instance ID. Subsequent runs reuse the same ID.

## Development Workflow

```bash
# Fast type/syntax check (no linking)
cargo check --workspace

# Check a single crate
cargo check -p trusty-store

# Run all tests
cargo test --workspace

# Run tests for one crate
cargo test -p trusty-embeddings

# Run the CLI
cargo run --bin trusty -- chat

# Run the daemon (foreground)
cargo run --bin trusty-daemon -- start --foreground

# Run the REST API
cargo run --bin trusty-api

# Watch for compile errors as you type (requires cargo-watch)
cargo watch -x check
```

### Log Levels

Set `TRUSTY_LOG_LEVEL` to `trace`, `debug`, `info`, `warn`, or `error`:

```bash
TRUSTY_LOG_LEVEL=debug cargo run --bin trusty -- chat
```

## Architecture Overview

The project is a Cargo workspace. Crates are layered bottom-up:

```
trusty-models        # Pure data types — no I/O, no logic
trusty-embeddings    # Local ONNX embeddings (fastembed) + BM25 (tantivy)
trusty-store         # Persistence: LanceDB (vectors), Kuzu (graph), SQLite
trusty-extractor     # LLM-based named entity recognition
trusty-email         # Gmail OAuth2 + sync
trusty-memory        # Memory recall and storage
trusty-chat          # Conversation engine, tool dispatch, RAG
trusty-core          # Config loading, error types, logging
trusty-daemon        # Background sync daemon + IPC server
trusty-api           # REST API (axum)
trusty-cli           # CLI (clap)
trusty-tui           # Terminal UI (ratatui)
trusty-telegram      # Telegram bot interface
```

See `docs/specs/` for detailed specifications for each layer.

## Testing

```bash
# All tests
cargo test --workspace

# A specific test by name
cargo test --workspace -- my_test_name

# With output (don't capture stdout)
cargo test --workspace -- --nocapture
```

Unit tests live alongside source files (`#[cfg(test)]` blocks). Integration tests are in
each crate's `tests/` directory where present.

## Data Directory

All persistent data lives in `~/.local/share/trusty-izzie/` by default (configurable via
`TRUSTY_DATA_DIR`):

```
~/.local/share/trusty-izzie/
├── instance.json          # Instance identity (generated on first run)
├── trusty.db              # SQLite: OAuth tokens, cursors, event queue
├── lance/                 # LanceDB: entity + memory vectors
└── kuzu/                  # Kuzu: knowledge graph
```

Do not commit this directory. It is excluded by `.gitignore`.

## Cargo Conventions

Always use workspace-inherited versions in crate `Cargo.toml` files:

```toml
[dependencies]
tokio = { workspace = true }
serde = { workspace = true }
```

Never pin a version directly — use `{ workspace = true }` and update `Cargo.toml` at the
workspace root.
