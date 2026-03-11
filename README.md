# trusty-izzie

A headless personal AI assistant that runs entirely on your local machine. It learns from your email and calendar, extracts entities and relationships, and gives you a conversational interface to your own professional context.

## What it is

trusty-izzie is a local-first AI assistant that:

- Syncs with your Gmail (OAuth2, read-only) and indexes sent emails
- Extracts people, companies, projects, and relationships using an LLM
- Builds a personal knowledge graph stored in Kuzu (local graph DB)
- Provides hybrid semantic + BM25 search across your memories
- Runs a background daemon for continuous email ingestion
- Exposes a REST API, CLI, and TUI for interaction

All data stays on your machine. Embeddings are generated locally via fastembed (ONNX). The only outbound calls are to OpenRouter for LLM inference and Google APIs for Gmail.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     User Interfaces                         │
│   trusty-cli (clap)   trusty-tui (ratatui)   trusty-api    │
│                                               (axum REST)   │
└──────────────────┬──────────────────┬────────────────────────┘
                   │                  │
┌──────────────────▼──────────────────▼────────────────────────┐
│                      trusty-chat                             │
│          Conversation engine, tool dispatch, RAG             │
└──────────────────┬──────────────────────────────────────────┘
                   │
┌──────────────────▼──────────────────────────────────────────┐
│                      trusty-core                            │
│          Config loading, error types, logging setup          │
└──────┬──────────────┬──────────────────────────────────────┘
       │              │
┌──────▼────┐  ┌──────▼──────────┐  ┌───────────────────────┐
│trusty-    │  │ trusty-memory   │  │ trusty-daemon         │
│extractor  │  │ (recall, store, │  │ (email sync loop,     │
│(LLM NER)  │  │  retrieval)     │  │  IPC server)          │
└──────┬────┘  └──────┬──────────┘  └───────────────────────┘
       │              │
┌──────▼──────────────▼────────────────────────────────────────┐
│                      trusty-store                            │
│   LanceDB (vectors)  Kuzu (graph)  SQLite (auth/config)      │
└──────────────────────────────────────────────────────────────┘
                   │
┌──────────────────▼──────────────────────────────────────────┐
│                   trusty-embeddings                         │
│       fastembed (local ONNX)  tantivy (BM25)  RRF fusion    │
└──────────────────────────────────────────────────────────────┘
                   │
┌──────────────────▼──────────────────────────────────────────┐
│                    trusty-models                            │
│         Pure data types: Entity, Memory, Chat, Email        │
└─────────────────────────────────────────────────────────────┘
```

## Prerequisites

- **Rust 1.75+** — install via [rustup](https://rustup.rs/)
- **macOS** (recommended) — Linux is experimental; Windows is unsupported
- **Google Cloud Console project** — for Gmail, Calendar, Tasks, and People API access
- **OpenRouter account** — for LLM inference ([openrouter.ai](https://openrouter.ai))
- **ngrok** (optional) — to expose localhost publicly for Telegram webhooks or remote OAuth callbacks
- **Telegram account** (optional) — only needed for the Telegram bot interface

## Google OAuth Setup

trusty-izzie uses Google OAuth 2.0 to read your Gmail and Calendar data. Follow these steps once before the first run:

1. Go to [Google Cloud Console](https://console.cloud.google.com/) and create a new project (or select an existing one).

2. Enable the following APIs under **APIs & Services → Library**:
   - Gmail API
   - Google Calendar API
   - Tasks API
   - People API

3. Go to **APIs & Services → Credentials → Create Credentials → OAuth client ID**.
   - Application type: **Desktop app** (simplest for local use)
   - Give it a name and click **Create**

4. Under **Authorized redirect URIs**, add:
   - `http://localhost:8080` (for local development)
   - Your public domain callback if using ngrok, e.g. `https://myngrok.ngrok.io/api/auth/google/callback`

5. Copy the **Client ID** and **Client Secret** into your `.env` file:
   ```
   GOOGLE_CLIENT_ID=...
   GOOGLE_CLIENT_SECRET=...
   ```

6. Run the authentication flow:
   ```bash
   cargo run --bin trusty -- auth google
   ```
   This opens your browser for consent. Tokens are stored locally in SQLite and refreshed automatically.

## Telegram Integration

The Telegram bot is optional. It gives you a mobile-friendly chat interface to the same AI assistant.

1. Open Telegram and search for **@BotFather**.
2. Send `/newbot`, follow the prompts, and copy the bot token.
3. Set the token in `.env`:
   ```
   TELEGRAM_BOT_TOKEN=your_bot_token_here
   ```
4. Set your public URL (required for webhook mode — use ngrok or a real domain):
   ```
   TRUSTY_PUBLIC_URL=https://your.public.domain
   ```
5. Build and start the bot:
   ```bash
   cargo build --release
   ./scripts/telegram-start.sh
   ```

Without `TRUSTY_PUBLIC_URL`, the bot falls back to long-polling mode (`--poll` flag), which works without a public domain.

## Quick Start

```bash
# 1. Build the project
cargo build --release

# 2. Copy and fill in environment variables
cp .env.example .env
$EDITOR .env  # Set OPENROUTER_API_KEY, GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET

# 3. Authenticate with Google (opens browser)
trusty auth google

# 4. Start the background daemon
trusty daemon start

# 5. Wait for initial email sync (check status)
trusty sync --force

# 6. Start chatting
trusty chat
```

## Crate Structure

| Crate | Role | Key Dependencies |
|---|---|---|
| `trusty-models` | Pure data types, no logic | serde, chrono, uuid |
| `trusty-embeddings` | Local embedding + BM25 search | fastembed, tantivy |
| `trusty-store` | Persistence layer | lancedb, kuzu, rusqlite |
| `trusty-extractor` | LLM-based entity extraction | reqwest, trusty-store |
| `trusty-email` | Gmail OAuth2 sync | oauth2, reqwest |
| `trusty-memory` | Memory recall and storage | trusty-store, trusty-embeddings |
| `trusty-chat` | Conversation engine | trusty-memory, trusty-extractor |
| `trusty-core` | Config, errors, logging | config, tracing, dotenvy |
| `trusty-daemon` | Background sync daemon | tokio, interprocess |
| `trusty-api` | REST API server | axum, tower |
| `trusty-cli` | Command-line interface | clap, interprocess |
| `trusty-tui` | Terminal UI | ratatui, crossterm |

## Configuration Reference

All settings live in `config/default.toml` and can be overridden with environment variables or via `trusty config set`.

| Key | Default | Description |
|---|---|---|
| `openrouter.chat_model` | `anthropic/claude-sonnet-4-5` | Model used for chat |
| `openrouter.extraction_model` | `mistralai/mistral-small-3.1-24b-instruct` | Model for NER extraction |
| `openrouter.embedding_model` | `all-MiniLM-L6-v2` | Local ONNX embedding model |
| `storage.data_dir` | `~/.local/share/trusty-izzie` | Root for all persistent data |
| `daemon.email_sync_interval_secs` | `1800` | How often to poll Gmail (30 min) |
| `daemon.ipc_socket` | `/tmp/trusty-izzie.sock` | Unix socket for CLI<->daemon IPC |
| `extraction.confidence_threshold` | `0.85` | Min confidence to store an entity |
| `extraction.min_occurrences` | `2` | Min times seen before storing |
| `extraction.max_relationships_per_email` | `3` | Cap relationships extracted per email |
| `chat.max_tool_iterations` | `5` | Max LLM tool call rounds per message |
| `chat.context_memory_limit` | `10` | Memories injected into context |
| `chat.context_entity_limit` | `15` | Entities injected into context |
| `api.port` | `3456` | REST API port |
| `api.host` | `127.0.0.1` | REST API bind address |

## CLI Commands

```
trusty chat [--session <id>]          Interactive chat session
trusty entity list [--type <type>]    List extracted entities
trusty entity search <query>          Semantic search over entities
trusty memory list                    List stored memories
trusty sync [--force]                 Trigger email sync
trusty daemon start|stop|status       Manage background daemon
trusty auth google                    Start Google OAuth flow
trusty config get <key>               Get a config value
trusty config set <key> <value>       Set a config value
```

## License

MIT
