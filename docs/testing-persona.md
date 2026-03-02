# Testing Persona — masa (bobmatnyc@gmail.com)

This document describes the real user persona for trusty-izzie and provides
a structured test script for verifying the system's knowledge, recall, and
conversational quality against the migrated email data.

---

## Identity

| Field | Value |
|-------|-------|
| Name | masa (Robert Matney) |
| Email | bobmatnyc@gmail.com |
| Instance ID | `42a923e9bd673e38` |
| Data migrated | 2026-03-01 from Weaviate (izzie2) |
| Entities | 6,774 |
| Memories | 29 |

---

## Data Provenance

All knowledge comes from **outgoing email only** (SENT folder), which means:

- **Persons** are real contacts masa has emailed
- **Companies** are organisations mentioned in professional correspondence
- **Projects** are work items masa has actively discussed
- **Tools/Technologies** are things masa has written about
- **Memories** are facts the system inferred from patterns across emails

The confidence threshold was **≥ 0.85**. The 266 entities below threshold
from the original izzie2 migration were excluded.

---

## CLI Test Script

Run these in order to verify the full pipeline. All `--test` invocations are
**read-only** — they call the LLM but save nothing.

### 1. Sanity checks

```bash
# Version and build info
trusty version

# Show all running processes
trusty status

# Confirm migrated data is visible
trusty entity list --limit 20
trusty entity list --type person --limit 20
trusty memory list
```

### 2. Entity recall (persons)

```bash
# Search for specific people masa works with
trusty entity search "alice"
trusty entity search "bob"
trusty entity search "anthropic"
trusty entity search "engineering"

# Should return entities with confidence ≥ 0.85 sourced from email headers
```

### 3. Memory recall

```bash
# Search memories for professional context
trusty memory search "project"
trusty memory search "meeting"
trusty memory search "deadline"
trusty memory search "launch"
```

### 4. Chat — dry-run (read-only, no saves)

These test LLM integration and context assembly without touching the DB.

```bash
# Basic presence check
trusty chat --test "Hi, are you there?"

# Context awareness — should reflect actual email relationships
trusty chat --test "Who are the people I work with most closely?"
trusty chat --test "What projects am I currently involved in?"
trusty chat --test "What tools and technologies do I use regularly?"

# Memory retrieval
trusty chat --test "What do you know about my work style and preferences?"

# Temporal awareness
trusty chat --test "What happened recently that I should follow up on?"
```

### 5. Session continuity

```bash
# Start a conversation (live — uses tokens, saves session)
trusty chat "Let's talk about my most important professional relationships."
trusty chat "Which of those people should I reach out to this week?"
trusty chat "Draft a brief catch-up message to the first person you mentioned."

# View the session
trusty session list
trusty session show <UUID-from-above>

# Clean up
trusty clear
```

### 6. Interaction log verification

```bash
# All commands above should have logged entries
cat ~/.local/share/trusty-izzie/interactions.jsonl | python3 -c "
import sys, json
for line in sys.stdin:
    d = json.loads(line)
    print(f'{d[\"ts\"]} {d[\"command\"]:20s} dur={d.get(\"duration_ms\",0)}ms')
"
```

---

## Expected Behaviour

### What should work

| Command | Expected |
|---------|----------|
| `entity list` | 6,774 entities visible from Kuzu/LanceDB |
| `entity search "google"` | Returns Company entities with Google affiliation |
| `memory list` | 29 memories shown, sorted by strength |
| `chat --test "hello"` | LLM responds with context-aware greeting |
| `chat --test "who do I know?"` | Names real contacts from entity store |
| `session list` | Lists sessions after any live chat |
| `trusty version` | Shows version, git hash, build date |

### What is read-only safe (`--test` flag)

- `chat --test "..."` — calls OpenRouter, shows response, **saves nothing**
- Prints: `[TEST MODE — read-only, nothing will be saved]`
- Shows `memoriesToSave` entries that *would* have been written
- Session is not persisted; `chat:current_session` not updated

### Known limitations

| Limitation | Reason |
|------------|--------|
| Kuzu graph empty on first run | Python-created single-file DB incompatible with Rust kuzu-rs directory format; graph rebuilds from email processing |
| `trusty sync` blocked | Gmail OAuth not yet completed — run `trusty auth` first |
| `trusty auth` opens browser | PKCE flow requires interactive browser step |
| Telegram requires pairing | Run `make telegram-pair` before `make telegram` |

---

## Authentication Flow

```bash
# One-time setup
trusty auth
# → opens https://accounts.google.com/o/oauth2/v2/auth in browser
# → you log in as bobmatnyc@gmail.com
# → Google redirects to http://localhost:8080/callback
# → tokens stored in ~/.local/share/trusty-izzie/trusty.db
# → "✓ Authenticated as bobmatnyc@gmail.com"

# Then pull email
trusty sync
# → fetches SENT messages since last history cursor
# → extracts entities and memories
# → Kuzu graph re-populates
```

---

## Full Automated Test

```bash
# Run all tests (dry-run only)
bash scripts/test-features.sh

# Run including live chat (costs ~$0.01 in OpenRouter tokens)
bash scripts/test-features.sh --chat

# Or via Make
make test-features
```
