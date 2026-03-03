# 09 — Agent Model

## Overview

The agent model gives trusty-izzie a long-term task execution system on top of its
existing event-driven architecture. Where chat interactions are ephemeral (request →
response in one turn), **agents** are autonomous workers that can run for minutes or
hours, produce persistent output, and be retried on failure.

An agent is defined by a Markdown file in `docs/agents/`. The daemon reads these
definitions, dispatches tasks to the appropriate LLM model, persists progress in the
`agent_tasks` SQLite table, and stores final output for retrieval via the REST API.

### Why It Exists

- Chat tools (e.g., `search_entities`, `recall_memory`) answer questions in <2 s.
- Some tasks need more time: synthesising research across many emails, writing a
  complete Python script, producing a structured report.
- These tasks should run in the background, not block a chat session.
- Results must survive daemon restarts and be queryable later.

---

## Agent Definition File Format

Agent definitions live in `docs/agents/{name}.md`. The filename stem (without `.md`)
is the canonical `agent_name` used in events and the database.

### Format

```
---
model: <openrouter model id>
max_runtime_mins: <integer>
description: <one-line human description>
---

# Instructions

<free-form Markdown instructions for the agent>
```

### Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `model` | string | yes | OpenRouter model ID (e.g. `anthropic/claude-opus-4-5`) |
| `max_runtime_mins` | integer | yes | Wall-clock timeout; daemon cancels the task after this |
| `description` | string | yes | One-line description shown in `GET /api/agents` |

The Markdown body below the front-matter is the full system prompt injected into the
LLM call. Keep it focused and action-oriented.

### Bundled Agents

| File | Agent Name | Model | Timeout |
|------|-----------|-------|---------|
| `researcher.md` | `researcher` | claude-opus-4-5 | 60 min |
| `script-writer.md` | `script-writer` | claude-opus-4-5 | 30 min |
| `summarizer.md` | `summarizer` | claude-sonnet-4-5 | 15 min |

### Adding a New Agent

1. Create `docs/agents/{name}.md` with valid YAML front-matter and instructions.
2. Restart the daemon (it re-reads the directory on startup).
3. The new agent is immediately available via `POST /api/tasks`.

---

## AgentRun Event Flow

```
Chat tool call
    │
    ▼
EventPayload::AgentRun {
    agent_name: "researcher",
    task_description: "Summarise all emails from Q1 2025 mentioning Project X",
    context: Some("…extra injected context…"),
}
    │
    ▼  (SqliteStore::enqueue_event)
event_queue  [status=pending, priority=3]
    │
    ▼  (daemon poll loop — SqliteStore::claim_next_event)
AgentRun handler
    ├── 1. Read docs/agents/{agent_name}.md
    ├── 2. Parse YAML front-matter → model, max_runtime_mins
    ├── 3. SqliteStore::create_agent_task(…)   [status=pending]
    ├── 4. SqliteStore::update_agent_task(…, "running", …)
    ├── 5. OpenRouter API call (streaming, timeout = max_runtime_mins)
    ├── 6a. Success → update_agent_task(…, "done", output=…)
    └── 6b. Failure → update_agent_task(…, "error", error=…)
              └── SqliteStore::fail_event(…) for retry if attempts < max_retries
```

### Priority

`AgentRun` events have `default_priority = 3` (below NeedsReauth=1, Reminder=2;
above EmailSync=4). Agent tasks are user-initiated work and should not starve
background sync.

### Retries

`AgentRun` has `default_max_retries = 1`. LLM tasks are expensive and usually fail
for deterministic reasons (bad prompt, context-length overflow). One retry is enough
to handle transient API errors.

---

## `agent_tasks` Table

```sql
CREATE TABLE IF NOT EXISTS agent_tasks (
    id               TEXT PRIMARY KEY,
    agent_name       TEXT NOT NULL,
    task_description TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'pending',
    model            TEXT,
    output           TEXT,
    error            TEXT,
    created_at       INTEGER NOT NULL,
    started_at       INTEGER,
    completed_at     INTEGER,
    parent_event_id  TEXT
);
CREATE INDEX IF NOT EXISTS idx_at_status ON agent_tasks(status, created_at DESC);
```

### Column Notes

| Column | Notes |
|--------|-------|
| `id` | UUIDv4 string generated at task creation |
| `agent_name` | Filename stem of the agent definition |
| `status` | `pending` → `running` → `done` \| `error` |
| `model` | Actual model used; may override agent-definition default |
| `output` | Full LLM response text on success |
| `error` | Error message string on failure |
| `created_at` | Unix timestamp (seconds) |
| `started_at` | Set when status transitions to `running` |
| `completed_at` | Set when status transitions to `done` or `error` |
| `parent_event_id` | The `event_queue.id` that triggered this task |

### Lifecycle

```
created_at set ──► [pending]
                      │
             daemon claims event
                      │
                   [running]  ◄── started_at set
                      │
          ┌───────────┴───────────┐
       success                 failure
          │                       │
       [done]                  [error]
   output = LLM text        error = message
   completed_at set         completed_at set
```

---

## `AgentTask` Rust Type

Defined in `trusty-models::agent`:

```rust
pub struct AgentTask {
    pub id: String,
    pub agent_name: String,
    pub task_description: String,
    pub status: String,
    pub model: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub parent_event_id: Option<String>,
}
```

---

## REST API

These endpoints will be implemented in `trusty-api` (Sprint 3+).

### List available agents

```
GET /api/agents
```

Reads `docs/agents/*.md` (path from `[agents] agents_dir` config), parses front-matter,
returns agent metadata.

Response:
```json
[
  {
    "name": "researcher",
    "description": "Deep research and multi-step analysis tasks",
    "model": "anthropic/claude-opus-4-5",
    "max_runtime_mins": 60
  }
]
```

### Submit a task

```
POST /api/tasks
Content-Type: application/json

{
  "agent_name": "researcher",
  "task_description": "Analyse all outgoing emails to Acme Corp in 2025",
  "context": null
}
```

Response `202 Accepted`:
```json
{ "task_id": "<uuid>" }
```

Internally: calls `SqliteStore::enqueue_event` with `EventPayload::AgentRun`, then
`SqliteStore::create_agent_task`.

### Get task status

```
GET /api/tasks/{task_id}
```

Returns the `AgentTask` struct as JSON. Poll until `status` is `done` or `error`.

### List tasks

```
GET /api/tasks?status=done&limit=20
```

Returns `Vec<AgentTask>`, newest first.

---

## Configuration

In `config/default.toml`:

```toml
[agents]
agents_dir = "docs/agents"   # relative to working dir, or absolute
```

The daemon and API server resolve `agents_dir` relative to the process working
directory at startup. Use an absolute path in production deployments.

---

## Implementation Notes

- Agent definition parsing uses the `---` YAML front-matter convention. A simple
  line-by-line parser suffices; no full YAML crate dependency needed.
- The daemon's `AgentRun` handler should enforce `max_runtime_mins` via
  `tokio::time::timeout`.
- `output` is stored verbatim. For very long outputs (>100 KB), consider truncating
  or storing in a separate file referenced by path.
- The `context` field in `EventPayload::AgentRun` is injected after the system
  prompt and before the user message, giving the caller a way to pass dynamic
  context (e.g. retrieved entity summaries) without modifying the agent definition.
