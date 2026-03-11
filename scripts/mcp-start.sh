#!/usr/bin/env bash
# scripts/mcp-start.sh - Start the Izzie MCP server (HTTP SSE mode) in the background.
#
# The MCP server exposes Izzie's data (contacts, calendar, tasks, memories,
# entities) as MCP tools on port 3458. Other apps can connect via HTTP SSE,
# or use the binary directly in stdio mode for Claude Desktop integration.
#
# Claude Desktop config (~/.config/claude/claude_desktop_config.json):
#   {
#     "mcpServers": {
#       "izzie": {
#         "command": "/path/to/trusty-mcp",
#         "args": ["--stdio"]
#       }
#     }
#   }
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

MCP_PID_FILE="/tmp/trusty-mcp.pid"
MCP_LOG_FILE="/tmp/trusty-mcp.log"
MCP_PORT="${TRUSTY_MCP_PORT:-3458}"

if [[ -x "$PROJECT_DIR/target/release/trusty-mcp" ]]; then
    MCP_BIN="$PROJECT_DIR/target/release/trusty-mcp"
elif [[ -x "$PROJECT_DIR/target/debug/trusty-mcp" ]]; then
    MCP_BIN="$PROJECT_DIR/target/debug/trusty-mcp"
else
    echo "✗ trusty-mcp binary not found."
    echo "  Run 'cargo build --release -p trusty-mcp' first."
    exit 1
fi

if [[ -f "$MCP_PID_FILE" ]]; then
    OLD_PID=$(cat "$MCP_PID_FILE")
    if kill -0 "$OLD_PID" 2>/dev/null; then
        echo "▸ trusty-mcp is already running (PID $OLD_PID, port $MCP_PORT)"
        exit 0
    fi
    rm -f "$MCP_PID_FILE"
fi

if [[ -f "$PROJECT_DIR/.env" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "$PROJECT_DIR/.env"
    set +a
fi

echo "▶ Starting trusty-mcp on port $MCP_PORT..."
echo "  Log → $MCP_LOG_FILE"
echo "  HTTP SSE: http://localhost:$MCP_PORT/mcp/sse"

export RUST_BACKTRACE=1
nohup "$MCP_BIN" --port "$MCP_PORT" \
    >> "$MCP_LOG_FILE" 2>&1 &
MCP_PID=$!
echo "$MCP_PID" > "$MCP_PID_FILE"

sleep 1
if kill -0 "$MCP_PID" 2>/dev/null; then
    echo "✓ trusty-mcp started (PID $MCP_PID)"
    echo "  SSE endpoint: http://localhost:$MCP_PORT/mcp/sse"
    echo "  tail -f $MCP_LOG_FILE  to follow logs"
else
    echo "✗ trusty-mcp failed to start — check $MCP_LOG_FILE"
    rm -f "$MCP_PID_FILE"
    exit 1
fi
