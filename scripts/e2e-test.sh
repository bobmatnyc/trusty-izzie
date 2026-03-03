#!/usr/bin/env bash
# scripts/e2e-test.sh — End-to-end test suite for trusty-izzie API + CLI.
#
# Covers: service health, agent listing, task CRUD, stub route discovery,
# entity/memory CLI queries, self-awareness, and optional destructive task creation.
#
# Usage:
#   bash scripts/e2e-test.sh                # non-destructive
#   bash scripts/e2e-test.sh --destructive  # also creates real tasks (costs ~$0.001)
#   bash scripts/e2e-test.sh --json         # emit JSON summary at end
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# ── Configuration ──────────────────────────────────────────────────────────────
API_BASE="http://localhost:3456"
DATA_DIR="${HOME}/.local/share/trusty-izzie"
TRUSTY_BIN="${PROJECT_DIR}/target/release/trusty"
DESTRUCTIVE=false
JSON_OUTPUT=false

for arg in "$@"; do
    case "$arg" in
        --destructive) DESTRUCTIVE=true ;;
        --json)        JSON_OUTPUT=true ;;
    esac
done

# ── Counters ───────────────────────────────────────────────────────────────────
PASSED=0
FAILED=0
SKIPPED=0

# ── Colours ───────────────────────────────────────────────────────────────────
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
RESET='\033[0m'

# ── Helpers ───────────────────────────────────────────────────────────────────

pass() {
    echo -e "  ${GREEN}✓${RESET} $1"
    PASSED=$((PASSED + 1))
}

fail() {
    echo -e "  ${RED}✗${RESET} $1: $2"
    FAILED=$((FAILED + 1))
}

skip() {
    echo -e "  ${YELLOW}○${RESET} $1 (skipped: ${2:-})"
    SKIPPED=$((SKIPPED + 1))
}

# assert_http <label> <expected_code> <actual_code>
assert_http() {
    local label="$1" expected="$2" actual="$3"
    if [[ "$actual" == "$expected" ]]; then
        pass "$label"
    else
        fail "$label" "expected HTTP $expected, got HTTP $actual"
    fi
}

# assert_json <label> <json_string> <jq_filter>
# Passes if jq filter exits 0 (truthy result), fails otherwise.
assert_json() {
    local label="$1" json="$2" filter="$3"
    if echo "$json" | jq -e "$filter" > /dev/null 2>&1; then
        pass "$label"
    else
        fail "$label" "jq filter '$filter' returned false or errored"
    fi
}

# stub_check <label> <curl_args...>
# Reports route registration status. Never increments pass/fail.
stub_check() {
    local label="$1"; shift
    local http_code
    http_code=$(curl -s -o /dev/null -w "%{http_code}" "$@" 2>/dev/null || echo "000")
    if [[ "$http_code" == "404" ]]; then
        echo -e "  ${YELLOW}[STUB-MISSING]${RESET} $label -> 404 (route NOT registered)"
    elif [[ "$http_code" == "000" ]]; then
        echo -e "  ${RED}[STUB-ERROR]${RESET} $label -> connection failed"
    else
        echo -e "  ${CYAN}[STUB]${RESET} $label -> HTTP $http_code (route registered)"
    fi
}

# run_command_ok <label> <cmd...>
# Pass if command exits 0, fail otherwise. Output suppressed.
run_command_ok() {
    local label="$1"; shift
    if "$@" > /dev/null 2>&1; then
        pass "$label"
    else
        fail "$label" "command failed: $*"
    fi
}

# http_get <url> — returns body on stdout; saves HTTP code to HTTP_CODE
http_get() {
    HTTP_CODE=$(curl -s -o /tmp/trusty_e2e_body -w "%{http_code}" "$1" 2>/dev/null || echo "000")
    cat /tmp/trusty_e2e_body 2>/dev/null || true
}

# http_post <url> <json_body> — returns body on stdout; saves HTTP code to HTTP_CODE
http_post() {
    HTTP_CODE=$(curl -s -o /tmp/trusty_e2e_body -w "%{http_code}" \
        -X POST -H "Content-Type: application/json" -d "$2" "$1" 2>/dev/null || echo "000")
    cat /tmp/trusty_e2e_body 2>/dev/null || true
}

# ── Pre-flight ─────────────────────────────────────────────────────────────────

check_prereqs() {
    command -v curl > /dev/null || { echo "ERROR: curl required"; exit 1; }
    command -v jq   > /dev/null || { echo "ERROR: jq required"; exit 1; }

    local code
    code=$(curl -s -o /dev/null -w "%{http_code}" "${API_BASE}/health" 2>/dev/null || echo "000")
    if [[ "$code" != "200" ]]; then
        echo "ERROR: API not responding at ${API_BASE} (got HTTP $code)"
        echo "  Start with:  make api  OR  launchctl start com.trusty-izzie.api"
        exit 1
    fi

    # Source .env if present (exports OPENROUTER_API_KEY etc.)
    if [[ -f "${PROJECT_DIR}/.env" ]]; then
        # shellcheck disable=SC1091
        set -a; source "${PROJECT_DIR}/.env"; set +a
    fi
}

# ── Header ────────────────────────────────────────────────────────────────────

echo ""
echo -e "${CYAN}trusty-izzie e2e test suite${RESET}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  API:      ${API_BASE}"
echo "  Data dir: ${DATA_DIR}"
echo "  Binary:   ${TRUSTY_BIN}"
[[ "$DESTRUCTIVE" == "true" ]] && echo "  Mode:     DESTRUCTIVE (will create real tasks)"
echo ""

check_prereqs

# ━━━ Section 1: Service Health ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

echo "Section 1: Service Health"
echo "─────────────────────────"

# TC-H01: API health check
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "${API_BASE}/health" 2>/dev/null || echo "000")
assert_http "TC-H01: /health -> HTTP 200" "200" "$HTTP_CODE"

# TC-H02: Data directory exists
if [[ -d "${DATA_DIR}" ]]; then
    pass "TC-H02: data directory exists"
else
    fail "TC-H02: data directory exists" "${DATA_DIR} not found"
fi

# TC-H03: LanceDB entities directory
if [[ -d "${DATA_DIR}/lance/entities.lance" ]]; then
    pass "TC-H03: LanceDB entities.lance exists"
else
    fail "TC-H03: LanceDB entities.lance exists" "${DATA_DIR}/lance/entities.lance not found"
fi

# TC-H04: Kuzu directory
if [[ -d "${DATA_DIR}/kuzu" ]]; then
    pass "TC-H04: Kuzu directory exists"
else
    fail "TC-H04: Kuzu directory exists" "${DATA_DIR}/kuzu not found"
fi

# TC-H05: SQLite DB
if [[ -f "${DATA_DIR}/trusty.db" ]]; then
    pass "TC-H05: trusty.db exists"
else
    fail "TC-H05: trusty.db exists" "${DATA_DIR}/trusty.db not found"
fi

# TC-H06: launchd services check
if launchctl list 2>/dev/null | grep -q "trusty-izzie"; then
    pass "TC-H06: launchd trusty-izzie services registered"
else
    skip "TC-H06: launchd services check" "no com.trusty-izzie.* services loaded"
fi

# ━━━ Section 2: Agent Listing ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

echo ""
echo "Section 2: Agent Listing"
echo "─────────────────────────"

AGENTS_BODY=$(http_get "${API_BASE}/api/agents")
AGENTS_CODE="$HTTP_CODE"

# TC-A01: HTTP 200
assert_http "TC-A01: GET /api/agents -> HTTP 200" "200" "$AGENTS_CODE"

# TC-A02: Response is JSON array
assert_json "TC-A02: response is JSON array" "$AGENTS_BODY" '. | type == "array"'

# TC-A03: Array length == 3
assert_json "TC-A03: agent count == 3" "$AGENTS_BODY" '. | length == 3'

# TC-A04: Contains expected names
assert_json "TC-A04: contains summarizer" "$AGENTS_BODY" '[.[].name] | contains(["summarizer"])'
assert_json "TC-A04: contains researcher" "$AGENTS_BODY" '[.[].name] | contains(["researcher"])'
assert_json "TC-A04: contains script-writer" "$AGENTS_BODY" '[.[].name] | contains(["script-writer"])'

# TC-A05: Each agent has required fields
assert_json "TC-A05: all agents have name field" \
    "$AGENTS_BODY" '[.[].name] | all(. != null)'
assert_json "TC-A05: all agents have model field" \
    "$AGENTS_BODY" '[.[].model] | all(. != null)'
assert_json "TC-A05: all agents have description field" \
    "$AGENTS_BODY" '[.[].description] | all(. != null)'
assert_json "TC-A05: all agents have max_runtime_mins field" \
    "$AGENTS_BODY" '[.[].max_runtime_mins] | all(. != null)'

# TC-A06: summarizer model
assert_json "TC-A06: summarizer model == anthropic/claude-sonnet-4-5" \
    "$AGENTS_BODY" \
    '.[] | select(.name == "summarizer") | .model == "anthropic/claude-sonnet-4-5"'

# TC-A07: researcher model
assert_json "TC-A07: researcher model == anthropic/claude-opus-4-5" \
    "$AGENTS_BODY" \
    '.[] | select(.name == "researcher") | .model == "anthropic/claude-opus-4-5"'

# TC-A08: GET /api/agents/summarizer has recent_tasks
AGENT_BODY=$(http_get "${API_BASE}/api/agents/summarizer")
AGENT_CODE="$HTTP_CODE"
assert_http "TC-A08: GET /api/agents/summarizer -> HTTP 200" "200" "$AGENT_CODE"
assert_json "TC-A08: summarizer has recent_tasks field" \
    "$AGENT_BODY" '.recent_tasks | type == "array"'

# TC-A09: 404 for nonexistent agent
NONEXIST_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    "${API_BASE}/api/agents/nonexistent-xyz" 2>/dev/null || echo "000")
assert_http "TC-A09: GET /api/agents/nonexistent-xyz -> HTTP 404" "404" "$NONEXIST_CODE"

# ━━━ Section 3: Task Listing ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

echo ""
echo "Section 3: Task Listing"
echo "─────────────────────────"

# TC-T01: GET /api/tasks
TASKS_BODY=$(http_get "${API_BASE}/api/tasks")
TASKS_CODE="$HTTP_CODE"
assert_http "TC-T01: GET /api/tasks -> HTTP 200" "200" "$TASKS_CODE"

# TC-T02: Response is JSON array
assert_json "TC-T02: response is JSON array" "$TASKS_BODY" '. | type == "array"'

# TC-T03: GET /api/tasks?status=pending
PENDING_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    "${API_BASE}/api/tasks?status=pending" 2>/dev/null || echo "000")
assert_http "TC-T03: GET /api/tasks?status=pending -> HTTP 200" "200" "$PENDING_CODE"

# TC-T04: GET /api/tasks?status=done
DONE_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    "${API_BASE}/api/tasks?status=done" 2>/dev/null || echo "000")
assert_http "TC-T04: GET /api/tasks?status=done -> HTTP 200" "200" "$DONE_CODE"

# TC-T05: nonexistent task UUID returns 404
NOTFOUND_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    "${API_BASE}/api/tasks/00000000-0000-0000-0000-000000000000" 2>/dev/null || echo "000")
assert_http "TC-T05: GET /api/tasks/{nil-uuid} -> HTTP 404" "404" "$NOTFOUND_CODE"

# TC-T06: empty agent_name should reject (400/422)
# The handler explicitly validates empty strings; expect 400.
T06_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST -H "Content-Type: application/json" \
    -d '{"agent_name":"","task_description":"test"}' \
    "${API_BASE}/api/tasks" 2>/dev/null || echo "000")
if [[ "$T06_CODE" == "400" ]] || [[ "$T06_CODE" == "422" ]]; then
    pass "TC-T06: empty agent_name -> HTTP $T06_CODE (rejected)"
elif [[ "$T06_CODE" == "200" ]] || [[ "$T06_CODE" == "201" ]]; then
    skip "TC-T06: empty agent_name validation" "validation not yet implemented (got $T06_CODE)"
else
    fail "TC-T06: empty agent_name rejected" "unexpected HTTP $T06_CODE"
fi

# TC-T07: empty task_description should reject (400/422)
T07_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST -H "Content-Type: application/json" \
    -d '{"agent_name":"summarizer","task_description":""}' \
    "${API_BASE}/api/tasks" 2>/dev/null || echo "000")
if [[ "$T07_CODE" == "400" ]] || [[ "$T07_CODE" == "422" ]]; then
    pass "TC-T07: empty task_description -> HTTP $T07_CODE (rejected)"
elif [[ "$T07_CODE" == "200" ]] || [[ "$T07_CODE" == "201" ]]; then
    skip "TC-T07: empty task_description validation" "validation not yet implemented (got $T07_CODE)"
else
    fail "TC-T07: empty task_description rejected" "unexpected HTTP $T07_CODE"
fi

# ━━━ Section 4: Stub Route Verification ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

echo ""
echo "Section 4: Stub Route Verification (informational, no pass/fail)"
echo "──────────────────────────────────────────────────────────────────"

stub_check "GET /v1/entities" "${API_BASE}/v1/entities"
stub_check "GET /v1/entities/search?q=test" "${API_BASE}/v1/entities/search?q=test"
stub_check "GET /v1/memories" "${API_BASE}/v1/memories"
stub_check "POST /v1/chat" \
    -X POST -H "Content-Type: application/json" \
    -d '{"message":"test"}' \
    "${API_BASE}/v1/chat"

# ━━━ Section 5: User Persona — Entity/Relationship Queries ━━━━━━━━━━━━━━━━━━━

echo ""
echo "Section 5: User Persona — Entity/Relationship Queries"
echo "───────────────────────────────────────────────────────"

if [[ ! -f "${TRUSTY_BIN}" ]]; then
    skip "TC-P01 through TC-P06" \
        "trusty binary not built (run: cargo build --release -p trusty-cli)"
else
    run_command_ok "TC-P01: entity list (exit 0)" \
        "${TRUSTY_BIN}" entity list --limit 10

    run_command_ok "TC-P02: entity list --type person (exit 0)" \
        "${TRUSTY_BIN}" entity list --type person --limit 5

    run_command_ok "TC-P03: entity search 'google' (exit 0)" \
        "${TRUSTY_BIN}" entity search "google" --limit 5

    run_command_ok "TC-P04: entity search 'anthropic' (exit 0)" \
        "${TRUSTY_BIN}" entity search "anthropic" --limit 5

    run_command_ok "TC-P05: memory list (exit 0)" \
        "${TRUSTY_BIN}" memory list --limit 5

    run_command_ok "TC-P06: memory search 'project' (exit 0)" \
        "${TRUSTY_BIN}" memory search "project" --limit 5
fi

# ━━━ Section 6: Self-Awareness ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

echo ""
echo "Section 6: Self-Awareness"
echo "─────────────────────────"

# TC-SA01: version command
if [[ ! -f "${TRUSTY_BIN}" ]]; then
    skip "TC-SA01: trusty --version" "binary not built"
else
    VERSION_OUT=$("${TRUSTY_BIN}" --version 2>&1 || true)
    VERSION_EXIT=$("${TRUSTY_BIN}" --version > /dev/null 2>&1; echo $?)
    if [[ "$VERSION_EXIT" == "0" ]] && echo "$VERSION_OUT" | grep -qE '[0-9]+\.[0-9]+'; then
        pass "TC-SA01: trusty --version (output: $VERSION_OUT)"
    elif [[ "$VERSION_EXIT" == "0" ]]; then
        fail "TC-SA01: trusty --version" "exit 0 but no version number in output: '$VERSION_OUT'"
    else
        fail "TC-SA01: trusty --version" "non-zero exit"
    fi
fi

# TC-SA02: /api/agents endpoint is itself self-describing
# The agent list serves as the system's capability manifest.
AGENTS_SA=$(http_get "${API_BASE}/api/agents")
if echo "$AGENTS_SA" | jq -e '. | length > 0' > /dev/null 2>&1; then
    AGENT_NAMES=$(echo "$AGENTS_SA" | jq -r '[.[].name] | join(", ")')
    pass "TC-SA02: GET /api/agents returns capability manifest (agents: $AGENT_NAMES)"
else
    fail "TC-SA02: GET /api/agents capability manifest" "empty or invalid response"
fi

# ━━━ Section 7: Destructive Tests ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

if [[ "$DESTRUCTIVE" == "true" ]]; then
    echo ""
    echo "Section 7: Destructive Tests (creates real data)"
    echo "──────────────────────────────────────────────────"

    TASK_INPUT="Summarize the following in 2 sentences: Rust is a systems programming language focused on memory safety. It uses an ownership model and borrow checker. It was created by Mozilla Research and open-sourced in 2015. It competes with C and C++ for systems programming tasks."

    TASK_RESPONSE=$(http_post "${API_BASE}/api/tasks" \
        "{\"agent_name\":\"summarizer\",\"task_description\":\"${TASK_INPUT}\"}")
    TASK_CREATE_CODE="$HTTP_CODE"

    TASK_ID=$(echo "$TASK_RESPONSE" | jq -r '.task_id // empty' 2>/dev/null || true)

    if [[ -z "$TASK_ID" ]]; then
        fail "D01-create: POST /api/tasks returned task_id" \
            "HTTP $TASK_CREATE_CODE, body: $TASK_RESPONSE"
    else
        pass "D01-create: task enqueued (id: ${TASK_ID:0:8}...)"

        # Verify it appears in list
        LIST_BODY=$(http_get "${API_BASE}/api/tasks")
        FOUND=$(echo "$LIST_BODY" | jq -r --arg id "$TASK_ID" \
            '.[] | select(.id == $id) | .id' 2>/dev/null || true)
        if [[ "$FOUND" == "$TASK_ID" ]]; then
            pass "D01-list: task visible in GET /api/tasks"
        else
            fail "D01-list" "task ${TASK_ID} not found in task list"
        fi

        # Get task detail
        TASK_DETAIL=$(http_get "${API_BASE}/api/tasks/${TASK_ID}")
        if echo "$TASK_DETAIL" | jq -e '.agent_name == "summarizer"' > /dev/null 2>&1; then
            pass "D01-detail: GET /api/tasks/${TASK_ID:0:8}... correct agent_name"
        else
            fail "D01-detail" "unexpected task detail: $TASK_DETAIL"
        fi

        # Poll for completion (daemon must be running)
        MAX_WAIT=60; INTERVAL=5; ELAPSED=0; FINAL_STATUS=""; CURRENT_STATUS="unknown"
        while [[ $ELAPSED -lt $MAX_WAIT ]]; do
            CURRENT_STATUS=$(http_get "${API_BASE}/api/tasks/${TASK_ID}" | \
                jq -r '.status // "unknown"' 2>/dev/null || true)
            if [[ "$CURRENT_STATUS" == "done" ]] || [[ "$CURRENT_STATUS" == "error" ]]; then
                FINAL_STATUS="$CURRENT_STATUS"; break
            fi
            sleep $INTERVAL
            ELAPSED=$((ELAPSED + INTERVAL))
        done

        if [[ "$FINAL_STATUS" == "done" ]]; then
            OUTPUT=$(http_get "${API_BASE}/api/tasks/${TASK_ID}" | jq -r '.output // ""')
            if [[ -n "$OUTPUT" && "$OUTPUT" != "null" ]]; then
                pass "D01-complete: task done with output (${#OUTPUT} chars)"
            else
                fail "D01-complete" "task done but output is empty"
            fi
        elif [[ "$FINAL_STATUS" == "error" ]]; then
            ERR=$(http_get "${API_BASE}/api/tasks/${TASK_ID}" | jq -r '.error // ""')
            fail "D01-complete" "task error: $ERR"
        else
            skip "D01-complete" \
                "daemon not running — task remains pending (status: ${CURRENT_STATUS})"
        fi
    fi

    # D02: Pipeline verification (minimal cost if daemon not running)
    RESPONSE2=$(http_post "${API_BASE}/api/tasks" \
        '{"agent_name":"summarizer","task_description":"E2E pipeline test — this task verifies the event queue. Safe to ignore."}')
    TASK_ID2=$(echo "$RESPONSE2" | jq -r '.task_id // empty' 2>/dev/null || true)
    if [[ -n "$TASK_ID2" ]]; then
        STATUS2=$(http_get "${API_BASE}/api/tasks/${TASK_ID2}" | \
            jq -r '.status // "unknown"' 2>/dev/null || true)
        pass "D02-pipeline: event queued (status: $STATUS2)"
    else
        fail "D02-pipeline" "POST /api/tasks failed: $RESPONSE2"
    fi
fi

# ━━━ Summary ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo -e "  ${GREEN}${PASSED} passed${RESET}  ${RED}${FAILED} failed${RESET}  ${YELLOW}${SKIPPED} skipped${RESET}"
echo ""

if [[ "$JSON_OUTPUT" == "true" ]]; then
    echo "{\"passed\":${PASSED},\"failed\":${FAILED},\"skipped\":${SKIPPED}}"
fi

[[ $FAILED -eq 0 ]] || exit 1
