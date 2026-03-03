# trusty-izzie E2E Test Suite

End-to-end tests that verify the running API and CLI behave correctly against the
local data store. Tests are read-only by default; destructive tests (real LLM calls)
are gated behind `--destructive`.

---

## Running the Tests

```bash
# Non-destructive (API must be running)
make test-e2e

# With destructive tests (creates real tasks, costs ~$0.001)
make test-e2e-all

# Emit JSON summary at the end
make test-e2e-json

# Direct invocation
bash scripts/e2e-test.sh [--destructive] [--json]
```

---

## Prerequisites

| Requirement | How to satisfy |
|------------|----------------|
| `curl`     | Usually pre-installed on macOS |
| `jq`       | `brew install jq` |
| API running | `make api` (port 3456) |
| `.env` file | Copy `.env.example` and populate secrets |

The pre-flight check will exit cleanly with an error message if any prerequisite
is missing. It does not run any tests before the API is confirmed reachable.

---

## What the Suite Covers

### Section 1: Service Health (non-destructive)
Verifies the process and all data stores are up:
- API responds HTTP 200 at `/health`
- `~/.local/share/trusty-izzie/` directory exists
- LanceDB `entities.lance` directory exists
- Kuzu graph directory exists
- SQLite `trusty.db` file exists
- launchd services registered (skipped if not installed)

### Section 2: Agent Listing (non-destructive)
Fully-implemented endpoints tested against live agent manifests in `docs/agents/`:
- `GET /api/agents` returns HTTP 200, JSON array of length 3
- All agents have `name`, `model`, `description`, `max_runtime_mins`
- `summarizer` model is `anthropic/claude-sonnet-4-5`
- `researcher` model is `anthropic/claude-opus-4-5`
- `GET /api/agents/summarizer` returns `recent_tasks` array
- `GET /api/agents/nonexistent-xyz` returns HTTP 404

### Section 3: Task Listing (non-destructive)
- `GET /api/tasks` and `?status=` filter return HTTP 200 JSON arrays
- Nil UUID returns HTTP 404
- Empty `agent_name` / `task_description` rejected with 400 (skipped if validation absent)

### Section 4: Stub Route Verification (informational only)
Reports route registration status for not-yet-implemented endpoints.
Results are `[STUB]`, `[STUB-MISSING]`, or `[STUB-ERROR]`. They never
increment pass/fail counters — they exist to track implementation progress.

Checked routes:
- `GET /v1/entities`
- `GET /v1/entities/search?q=test`
- `GET /v1/memories`
- `POST /v1/chat`

### Section 5: User Persona — Entity/Relationship Queries (non-destructive)
Skipped if `target/release/trusty` is not built (`cargo build --release -p trusty-cli`).

Runs CLI commands and checks exit code only:
- `entity list` and `entity list --type person`
- `entity search "google"` — Google appears in most professional sent email
- `entity search "anthropic"` — Anthropic is the LLM provider in the toolchain
- `memory list` and `memory search "project"`

Rationale for Google/Anthropic: these are high-signal entities expected in any
professional AI developer's outgoing email corpus. Testing entity search with
them validates that the LanceDB/tantivy hybrid search pipeline returns results
without error, not that specific records exist.

### Section 6: Self-Awareness (non-destructive)
- `trusty --version` exits 0 and output contains a version number
- `GET /api/agents` returns a non-empty capability manifest

### Section 7: Destructive Tests (only with `--destructive`)
Creates real agent tasks via the event queue. Estimated cost: ~$0.001.

- **D01**: Creates a summarizer task with static Rust-description text, verifies
  the task appears in `GET /api/tasks`, checks `GET /api/tasks/{id}` detail,
  then polls up to 60 seconds for completion (skipped if daemon not running).
- **D02**: Creates a second pipeline-verification task; confirms event enqueue
  works independently of daemon availability.

Neither destructive test uses PII — the task input text is a static description
of the Rust programming language.

---

## Exit Codes

| Code | Meaning |
|------|---------|
| `0`  | All tests passed (skips do not count as failures) |
| `1`  | One or more tests failed |
| `1`  | Pre-flight check failed (API not running, missing tool) |
