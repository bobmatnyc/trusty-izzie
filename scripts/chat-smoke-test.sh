#!/usr/bin/env bash
# scripts/chat-smoke-test.sh — Chat response smoke-test for DEV and PROD instances.
#
# Validates that chat responses are natural language, not raw JSON and not
# contaminated with internal "[Tool calls: ...]" or "⚠️ RESPOND WITH JSON ONLY"
# markers.
#
# Run this BEFORE handing off any fix to the user.
#
# Usage:
#   bash scripts/chat-smoke-test.sh                  # test both DEV and PROD
#   bash scripts/chat-smoke-test.sh --dev-only        # skip PROD
#   bash scripts/chat-smoke-test.sh --prod-only       # skip DEV (don't start it)
#   bash scripts/chat-smoke-test.sh --no-stop         # leave DEV running after tests
#
# DEV = port 3458  (TRUSTY_ENV=dev)
# PROD = port 3456 (TRUSTY_ENV=prod)
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BIN="${PROJECT_DIR}/target/release/trusty-telegram"

DEV_PORT=3458
PROD_PORT=3456
DEV_BASE="http://localhost:${DEV_PORT}"
PROD_BASE="http://localhost:${PROD_PORT}"

DEV_ONLY=false
PROD_ONLY=false
NO_STOP=false
DEV_PID=""

for arg in "$@"; do
    case "$arg" in
        --dev-only)  DEV_ONLY=true  ;;
        --prod-only) PROD_ONLY=true ;;
        --no-stop)   NO_STOP=true   ;;
    esac
done

# ── Colours ───────────────────────────────────────────────────────────────────
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

PASSED=0
FAILED=0
INSTANCE_FAILED=false
HTTP_CODE="000"

pass() { echo -e "  ${GREEN}✓${RESET} $1"; PASSED=$((PASSED + 1)); }
fail() { echo -e "  ${RED}✗${RESET} $1"; echo -e "    ${RED}→ $2${RESET}"; FAILED=$((FAILED + 1)); INSTANCE_FAILED=true; }

# ── Helpers ───────────────────────────────────────────────────────────────────

# wait_healthy <base_url> <timeout_secs> — returns 0 when /health is 200
wait_healthy() {
    local base="$1" timeout="$2" elapsed=0
    while [[ $elapsed -lt $timeout ]]; do
        local code
        code=$(curl -s -o /dev/null -w "%{http_code}" "${base}/health" 2>/dev/null || echo "000")
        [[ "$code" == "200" ]] && return 0
        sleep 1
        elapsed=$((elapsed + 1))
    done
    return 1
}

# chat_send <base_url> <message> [session_id]
# MUST be called WITHOUT command substitution so HTTP_CODE assignment reaches
# the caller's scope. Body is written to /tmp/trusty_chat_smoke.
# Usage:
#   chat_send "$base" "hello"
#   body=$(cat /tmp/trusty_chat_smoke)
chat_send() {
    local base="$1" msg="$2" sid="${3:-}"
    local payload
    if [[ -n "$sid" ]]; then
        payload=$(printf '{"message":"%s","session_id":"%s"}' "$msg" "$sid")
    else
        payload=$(printf '{"message":"%s"}' "$msg")
    fi
    # Clear temp files so stale data never bleeds between calls
    rm -f /tmp/trusty_chat_headers /tmp/trusty_chat_smoke
    # Use -D to dump response headers. curl exit code is ignored — server may
    # RST the connection after a valid response and cause non-zero exit.
    curl -s \
        -D /tmp/trusty_chat_headers \
        -o /tmp/trusty_chat_smoke \
        -X POST -H "Content-Type: application/json" \
        --max-time 90 \
        -d "$payload" \
        "${base}/chat" 2>/dev/null || true
    # Extract status from header line "HTTP/1.1 200 OK"
    HTTP_CODE=$(awk 'NR==1{print $2}' /tmp/trusty_chat_headers 2>/dev/null || true)
    # Fallback: if -D file wasn't populated, infer from body
    if [[ -z "$HTTP_CODE" ]]; then
        if jq -e '.reply' /tmp/trusty_chat_smoke > /dev/null 2>&1; then
            HTTP_CODE="200"
        else
            HTTP_CODE="000"
        fi
    fi
}

# assert_reply <label> <reply_json>
# Checks: HTTP 200, reply field present, not raw JSON blob, no internal markers
assert_reply() {
    local label="$1" raw="$2"

    # 1) HTTP status
    if [[ "$HTTP_CODE" != "200" ]]; then
        fail "$label" "HTTP ${HTTP_CODE} (expected 200). Body: ${raw:0:200}"
        return
    fi

    # 2) Parse reply field
    local reply
    reply=$(echo "$raw" | jq -r '.reply // empty' 2>/dev/null || true)
    if [[ -z "$reply" ]]; then
        fail "$label" "No 'reply' field in response. Body: ${raw:0:200}"
        return
    fi

    # 3) Not an empty stub
    if [[ ${#reply} -lt 10 ]]; then
        fail "$label" "Reply suspiciously short (${#reply} chars): '${reply}'"
        return
    fi

    # 4) Not raw JSON (starts with '{')
    if echo "$reply" | grep -qE '^\s*\{'; then
        fail "$label" "Reply is raw JSON! First 200 chars: ${reply:0:200}"
        return
    fi

    # 5) No [Tool calls: ...] contamination
    if echo "$reply" | grep -qF '[Tool calls:'; then
        fail "$label" "Reply contains '[Tool calls:' marker: ${reply:0:200}"
        return
    fi

    # 6) No "RESPOND WITH JSON ONLY" leakage
    if echo "$reply" | grep -qi 'RESPOND WITH JSON'; then
        fail "$label" "Reply contains internal JSON-forcing instruction: ${reply:0:200}"
        return
    fi

    # 7) No raw JSON field names leaking into reply
    if echo "$reply" | grep -qE '"reply"\s*:'; then
        fail "$label" "Reply contains raw JSON key '\"reply\":': ${reply:0:200}"
        return
    fi

    # 8) No triple-backtick JSON block (sometimes LLM emits ```json {...}```)
    if echo "$reply" | grep -qE '^```(json)?\s*\{'; then
        fail "$label" "Reply is a fenced JSON block: ${reply:0:200}"
        return
    fi

    local preview="${reply:0:120}"
    pass "${label} — \"${preview}...\""
}

# ── Test probes ───────────────────────────────────────────────────────────────
# These messages deliberately trigger tool calls so we can verify the
# tool call/result pairs don't contaminate the final reply.

run_chat_probes() {
    local base="$1" label="$2"
    INSTANCE_FAILED=false

    echo -e "\n${BOLD}${label}${RESET} (${base})"
    echo "────────────────────────────────────────────"

    # Probe 1: Self-awareness — triggers get_izzie_status (fast, no external API)
    echo "  Sending: 'what is your current status?'"
    chat_send "$base" "what is your current status?"       # sets HTTP_CODE, body in /tmp/trusty_chat_smoke
    local body1; body1=$(cat /tmp/trusty_chat_smoke 2>/dev/null || true)
    assert_reply "P1 get_izzie_status — natural language reply" "$body1"

    # Probe 2: Calendar query — the original bug trigger
    echo "  Sending: 'what do i have on my calendar this week?'"
    chat_send "$base" "what do i have on my calendar this week?"
    local body2; body2=$(cat /tmp/trusty_chat_smoke 2>/dev/null || true)
    assert_reply "P2 get_calendar_events — natural language reply" "$body2"

    # Probe 3: Simple no-tool follow-up in same session — tests session not poisoned
    local session_id
    session_id=$(echo "$body2" | jq -r '.session_id // empty' 2>/dev/null || true)
    echo "  Sending: 'thanks' (same session — checks session not poisoned)"
    if [[ -n "$session_id" ]]; then
        chat_send "$base" "thanks" "$session_id"
        local body3; body3=$(cat /tmp/trusty_chat_smoke 2>/dev/null || true)
        assert_reply "P3 follow-up same session — no JSON contamination" "$body3"
    else
        echo "  (skipping P3 — no session_id from P2)"
    fi

    # Probe 4: Email query — another common tool trigger
    echo "  Sending: 'who have i emailed recently?'"
    chat_send "$base" "who have i emailed recently?"
    local body4; body4=$(cat /tmp/trusty_chat_smoke 2>/dev/null || true)
    assert_reply "P4 list_emails — natural language reply" "$body4"

    if [[ "$INSTANCE_FAILED" == "true" ]]; then
        echo -e "\n  ${RED}${BOLD}INSTANCE FAILED — do not promote to prod${RESET}"
        return 1
    else
        echo -e "\n  ${GREEN}${BOLD}All probes passed ✓${RESET}"
        return 0
    fi
}

# ── Start DEV instance ────────────────────────────────────────────────────────

DEV_DATA_DIR="${HOME}/.local/share/trusty-izzie-dev"
DEV_CONFIG_DIR="${HOME}/.config/trusty-izzie-dev"
DEV_CONFIG_FILE="${DEV_CONFIG_DIR}/config.env"

bootstrap_dev_env() {
    # Create DEV data dir (Kuzu/LanceDB will self-initialise on first run)
    mkdir -p "${DEV_DATA_DIR}/kuzu" "${DEV_DATA_DIR}/lance"

    # Remove stale Kuzu lock (left behind when a previous server was SIGKILL'd)
    rm -f "${DEV_DATA_DIR}/kuzu/.lock"

    # Create minimal DEV config if absent (so the binary doesn't fall back to
    # the prod .env which points at the prod data dir)
    if [[ ! -f "$DEV_CONFIG_FILE" ]]; then
        mkdir -p "$DEV_CONFIG_DIR"
        # Copy the project .env as base, then override the instance-specific keys
        local project_env="${PROJECT_DIR}/.env"
        if [[ -f "$project_env" ]]; then
            cp "$project_env" "$DEV_CONFIG_FILE"
        fi
        # Ensure dev-specific overrides are present
        {
            echo ""
            echo "# DEV instance overrides (written by chat-smoke-test.sh)"
            echo "TRUSTY_ENV=dev"
            echo "TRUSTY_DATA_DIR=${DEV_DATA_DIR}"
            echo "TRUSTY_API_PORT=${DEV_PORT}"
        } >> "$DEV_CONFIG_FILE"
        chmod 600 "$DEV_CONFIG_FILE"
        echo "  Created ${DEV_CONFIG_FILE}"
    fi
}

start_dev() {
    # Check if already running
    if wait_healthy "$DEV_BASE" 2; then
        echo "  DEV already running on :${DEV_PORT}"
        DEV_PID="existing"
        return 0
    fi

    if [[ ! -x "$BIN" ]]; then
        echo -e "${RED}ERROR: binary not found: ${BIN}${RESET}"
        echo "  Build with: cargo build --release -p trusty-telegram"
        exit 1
    fi

    bootstrap_dev_env

    echo "  Starting DEV server on :${DEV_PORT}..."
    # TRUSTY__INSTANCE__ENV=dev  → config crate reads instance.env="dev"
    #   → load_config dev override fires → data_dir = trusty-izzie-dev
    # TRUSTY_ENV=dev              → binary loads ~/.config/trusty-izzie-dev/config.env
    TRUSTY_ENV=dev \
    TRUSTY__INSTANCE__ENV=dev \
        "$BIN" start --http-only --port "$DEV_PORT" \
        > /tmp/trusty-dev-smoke.log 2>&1 &
    DEV_PID=$!

    if wait_healthy "$DEV_BASE" 90; then
        echo "  DEV server up (PID ${DEV_PID})"
        return 0
    else
        echo -e "${RED}  DEV server failed to start within 90s. Full logs:${RESET}"
        cat /tmp/trusty-dev-smoke.log 2>/dev/null || true
        kill "$DEV_PID" 2>/dev/null || true
        exit 1
    fi
}

stop_dev() {
    if [[ -n "$DEV_PID" && "$DEV_PID" != "existing" ]]; then
        echo "  Stopping DEV server (PID ${DEV_PID})"
        kill "$DEV_PID" 2>/dev/null || true
        wait "$DEV_PID" 2>/dev/null || true
    fi
}

# ── Main ──────────────────────────────────────────────────────────────────────

echo ""
echo -e "${CYAN}${BOLD}trusty-izzie chat smoke test${RESET}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Validates: responses are natural language"
echo "  Checks for: JSON blobs, [Tool calls:] markers, RESPOND WITH JSON"
echo "  DEV:  ${DEV_BASE}"
echo "  PROD: ${PROD_BASE}"
echo ""

OVERALL_OK=true

# ── DEV ───────────────────────────────────────────────────────────────────────
if [[ "$PROD_ONLY" == "false" ]]; then
    echo "Starting DEV instance..."
    start_dev

    if ! run_chat_probes "$DEV_BASE" "DEV instance (:${DEV_PORT})"; then
        OVERALL_OK=false
        echo -e "${RED}  DEV FAILED — stopping. Fix issues before testing PROD.${RESET}"
        [[ "$NO_STOP" == "false" ]] && stop_dev
        echo ""
        echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
        echo -e "  ${RED}${BOLD}SMOKE TEST FAILED${RESET} — ${FAILED} check(s) failed, ${PASSED} passed"
        echo ""
        exit 1
    fi

    [[ "$NO_STOP" == "false" ]] && stop_dev
fi

# ── PROD ──────────────────────────────────────────────────────────────────────
if [[ "$DEV_ONLY" == "false" ]]; then
    if ! wait_healthy "$PROD_BASE" 3; then
        echo -e "${YELLOW}  PROD not running on :${PROD_PORT} — skipping PROD probes${RESET}"
    else
        if ! run_chat_probes "$PROD_BASE" "PROD instance (:${PROD_PORT})"; then
            OVERALL_OK=false
        fi
    fi
fi

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
if [[ "$OVERALL_OK" == "true" ]]; then
    echo -e "  ${GREEN}${BOLD}ALL CLEAR${RESET} — ${PASSED} probes passed, ${FAILED} failed"
    echo -e "  ${GREEN}Safe to hand off to user.${RESET}"
else
    echo -e "  ${RED}${BOLD}SMOKE TEST FAILED${RESET} — ${FAILED} check(s) failed, ${PASSED} passed"
    echo -e "  ${RED}Do NOT hand off. Fix issues and re-run.${RESET}"
fi
echo ""

[[ "$OVERALL_OK" == "true" ]] || exit 1
