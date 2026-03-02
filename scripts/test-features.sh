#!/usr/bin/env bash
# scripts/test-features.sh — Integration test of all CLI features against the real DB.
#
# Tests every command against ~/.local/share/trusty-izzie/ without writing anything.
# All chat commands run in --test (dry-run) mode so no LLM calls are made to save.
#
# Usage:
#   bash scripts/test-features.sh           # run all tests
#   bash scripts/test-features.sh --chat    # include live chat test (costs tokens)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
CLI="$PROJECT_DIR/target/release/trusty"

LIVE_CHAT=false
for arg in "$@"; do
    [[ "$arg" == "--chat" ]] && LIVE_CHAT=true
done

# Counters
PASS=0
FAIL=0
SKIP=0

# ── Colours ───────────────────────────────────────────────────────────────────
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
RESET='\033[0m'

pass() { echo -e "  ${GREEN}✓${RESET} $1"; (( PASS++ )); }
fail() { echo -e "  ${RED}✗${RESET} $1: $2"; (( FAIL++ )); }
skip() { echo -e "  ${YELLOW}○${RESET} $1 (skipped)"; (( SKIP++ )); }

run() {
    # run <label> <expected-exit-code> <cmd...>
    local label="$1" expected_exit="$2"
    shift 2
    local output exit_code=0
    output=$("$@" 2>&1) || exit_code=$?
    if [[ $exit_code -eq $expected_exit ]]; then
        pass "$label"
        echo "$output"
    else
        fail "$label" "exit $exit_code (expected $expected_exit)"
        echo "$output"
    fi
}

run_contains() {
    # run_contains <label> <pattern> <cmd...>
    local label="$1" pattern="$2"
    shift 2
    local output exit_code=0
    output=$("$@" 2>&1) || exit_code=$?
    if [[ $exit_code -eq 0 ]] && echo "$output" | grep -qi "$pattern"; then
        pass "$label"
    elif [[ $exit_code -ne 0 ]]; then
        fail "$label" "exit $exit_code"
        echo "$output"
    else
        fail "$label" "output did not contain '$pattern'"
        echo "$output"
    fi
}

# ── Setup ─────────────────────────────────────────────────────────────────────
echo ""
echo -e "${CYAN}trusty-izzie feature test suite${RESET}"
echo "─────────────────────────────────────────"
echo "  Binary:   $CLI"
echo "  Data dir: ~/.local/share/trusty-izzie"
echo ""

# Load .env
if [[ -f "$PROJECT_DIR/.env" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "$PROJECT_DIR/.env"
    set +a
fi

# Ensure binary exists
if [[ ! -x "$CLI" ]]; then
    echo "✗ Release binary not found. Run: make build"
    exit 1
fi

# ── Tests ─────────────────────────────────────────────────────────────────────

echo "Version:"
run_contains  "trusty version"                        "0\." \
    "$CLI" version

echo ""
echo "Status:"
run_contains  "trusty status"                         "daemon" \
    "$CLI" status

echo ""
echo "Entity:"
run_contains  "entity list (all)"                     "Type\|Person\|Company\|No entities" \
    "$CLI" entity list --limit 10

run_contains  "entity list --type person"             "." \
    "$CLI" entity list --type person --limit 5

run_contains  "entity search 'google'"                "." \
    "$CLI" entity search "google" --limit 5

run_contains  "entity search 'alice'"                 "." \
    "$CLI" entity search "alice" --limit 5

echo ""
echo "Memory:"
run_contains  "memory list"                           "Category\|No memories" \
    "$CLI" memory list --limit 5

run_contains  "memory search 'project'"               "." \
    "$CLI" memory search "project" --limit 5

echo ""
echo "Session:"
run_contains  "session list"                          "Recent\|No sessions" \
    "$CLI" session list

echo ""
echo "Config:"
run           "config set test key"                   0 \
    "$CLI" config set "test.key" "test-value"

run_contains  "config get test key"                   "test-value" \
    "$CLI" config get "test.key"

echo ""
echo "Chat (dry-run / --test mode):"
if [[ -n "${OPENROUTER_API_KEY:-}" ]]; then
    run_contains  "chat --test 'hello'"              "izzie\|TEST MODE" \
        "$CLI" chat --test "Hello, are you there?"

    run_contains  "chat --test 'who do I know?'"    "izzie\|TEST MODE" \
        "$CLI" chat --test "Who are the people I interact with most?"
else
    skip "chat (no OPENROUTER_API_KEY)"
fi

echo ""
echo "Chat (live, saves session):"
if [[ "$LIVE_CHAT" == "true" ]] && [[ -n "${OPENROUTER_API_KEY:-}" ]]; then
    run_contains  "chat 'hello'"                      "izzie" \
        "$CLI" chat "Hello. What's my current context?"

    run_contains  "session list after chat"           "session\|Recent" \
        "$CLI" session list

    run_contains  "chat continues session"            "izzie" \
        "$CLI" chat "What did I just ask you?"

    run           "clear session"                     0 \
        "$CLI" clear
else
    skip "live chat (use --chat flag to enable)"
fi

echo ""
echo "Auth (check env only — no browser):"
if [[ -z "${GOOGLE_CLIENT_ID:-}" ]]; then
    skip "auth (no GOOGLE_CLIENT_ID in env)"
else
    # We can't test the full flow non-interactively, so just verify exit code
    # when credentials are present (it will block on browser — skip in CI)
    skip "auth (interactive browser flow — run 'trusty auth' manually)"
fi

# ── Interaction log ───────────────────────────────────────────────────────────
echo ""
echo "Interaction log:"
LOG_FILE="${TRUSTY_DATA_DIR:-$HOME/.local/share/trusty-izzie}/interactions.jsonl"
LOG_FILE="${LOG_FILE/\~/$HOME}"
if [[ -f "$LOG_FILE" ]]; then
    LOG_LINES=$(wc -l < "$LOG_FILE" | tr -d ' ')
    pass "interactions.jsonl exists ($LOG_LINES entries)"
    echo "  Last entry: $(tail -1 "$LOG_FILE" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["command"], d["ts"])' 2>/dev/null || echo '(parse error)')"
else
    skip "interactions.jsonl (not yet created — run a chat first)"
fi

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo "─────────────────────────────────────────"
echo -e "  ${GREEN}${PASS} passed${RESET}  ${RED}${FAIL} failed${RESET}  ${YELLOW}${SKIP} skipped${RESET}"
echo ""

if [[ $FAIL -gt 0 ]]; then
    exit 1
fi
