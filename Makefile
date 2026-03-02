# trusty-izzie Makefile
# All targets are thin wrappers around scripts/ for Claude Code / claude-mpm compatibility.

SHELL        := /bin/bash
PROJECT      := trusty-izzie
DAEMON_BIN   := target/release/trusty-daemon
API_BIN      := target/release/trusty-api
CLI_BIN      := target/release/trusty
TG_BIN       := target/release/trusty-telegram
PID_FILE     := /tmp/trusty-daemon.pid
API_PID_FILE := /tmp/trusty-api.pid
LOG_FILE     := /tmp/trusty-daemon.log
API_LOG_FILE := /tmp/trusty-api.log

.DEFAULT_GOAL := help

# ── Help ──────────────────────────────────────────────────────────────────────

.PHONY: help
help:
	@echo ""
	@echo "trusty-izzie — local-first personal AI assistant"
	@echo ""
	@echo "Build:"
	@echo "  make build          Build all binaries (release)"
	@echo "  make build-dev      Build all binaries (dev / fast)"
	@echo "  make check          cargo check workspace (fast syntax check)"
	@echo ""
	@echo "Daemon:"
	@echo "  make run            Build (release) then start daemon in background"
	@echo "  make run-dev        Build (dev) then start daemon in background"
	@echo "  make stop           Stop daemon (and API if running)"
	@echo "  make status         Show daemon + API process status"
	@echo "  make logs           Tail daemon logs (Ctrl-C to exit)"
	@echo ""
	@echo "API Server:"
	@echo "  make api            Start the REST API server (port 3456)"
	@echo "  make api-stop       Stop the REST API server"
	@echo "  make api-logs       Tail REST API logs"
	@echo ""
	@echo "Chat (CLI):"
	@echo "  make chat           Interactive chat via CLI (builds first if needed)"
	@echo ""
	@echo "Migration:"
	@echo "  make migrate        Add canonical id column to LanceDB tables (one-time)"
	@echo ""
	@echo "Email Sync:"
	@echo "  make sync           Trigger an immediate Gmail sync"
	@echo "  make auth           Run Google OAuth2 login flow"
	@echo ""
	@echo "Telegram:"
	@echo "  make telegram-pair  Pair a Telegram bot token (prompts interactively)"
	@echo "  make telegram       Start the Telegram bot"
	@echo "  make telegram-stop  Stop the Telegram bot"
	@echo ""
	@echo "Release:"
	@echo "  make version-patch  Bump patch version (0.1.0 → 0.1.1) and commit"
	@echo "  make version-minor  Bump minor version (0.1.0 → 0.2.0) and commit"
	@echo "  make version-major  Bump major version (0.1.0 → 1.0.0) and commit"
	@echo "  make tag            Tag current version without bumping"
	@echo "  make release        Patch bump + build + tag + push"
	@echo "  make release-minor  Minor bump + build + tag + push"
	@echo ""
	@echo "Testing:"
	@echo "  make test-features       Test all CLI features against real DB (dry-run)"
	@echo "  make test-features-chat  Same + live chat (uses OpenRouter tokens)"
	@echo ""
	@echo "Dev:"
	@echo "  make test           Run all unit tests"
	@echo "  make clippy         Run clippy (warnings as errors)"
	@echo "  make fmt            Run cargo fmt (auto-format)"
	@echo "  make ngrok          Start ngrok tunnel → izzie.ngrok.dev"
	@echo "  make clean          cargo clean"
	@echo ""

# ── Build ─────────────────────────────────────────────────────────────────────

.PHONY: build
build:
	@bash scripts/build.sh release

.PHONY: build-dev
build-dev:
	@bash scripts/build.sh dev

.PHONY: check
check:
	cargo check --workspace

# ── Daemon ────────────────────────────────────────────────────────────────────

.PHONY: run
run: build
	@bash scripts/daemon-start.sh release

.PHONY: run-dev
run-dev: build-dev
	@bash scripts/daemon-start.sh dev

.PHONY: stop
stop:
	@bash scripts/daemon-stop.sh

.PHONY: status
status:
	@bash scripts/status.sh

.PHONY: logs
logs:
	@tail -f "$(LOG_FILE)"

# ── API Server ────────────────────────────────────────────────────────────────

.PHONY: api
api: build
	@bash scripts/api-start.sh

.PHONY: api-stop
api-stop:
	@bash scripts/api-stop.sh

.PHONY: api-logs
api-logs:
	@tail -f "$(API_LOG_FILE)"

# ── Chat ──────────────────────────────────────────────────────────────────────

.PHONY: chat
chat:
	@bash scripts/chat.sh

# ── Email ─────────────────────────────────────────────────────────────────────

.PHONY: migrate
migrate:
	@bash scripts/migrate-lance-schema.sh

.PHONY: sync
sync:
	@bash scripts/sync.sh

.PHONY: auth
auth:
	@bash scripts/auth.sh

# ── Telegram ──────────────────────────────────────────────────────────────────

.PHONY: telegram-pair
telegram-pair:
	@bash scripts/telegram-pair.sh

.PHONY: telegram
telegram: build
	@bash scripts/telegram-start.sh

.PHONY: telegram-stop
telegram-stop:
	@bash scripts/telegram-stop.sh

# ── Release / versioning ──────────────────────────────────────────────────────

.PHONY: version-patch
version-patch:
	@bash scripts/version-bump.sh patch

.PHONY: version-minor
version-minor:
	@bash scripts/version-bump.sh minor

.PHONY: version-major
version-major:
	@bash scripts/version-bump.sh major

.PHONY: tag
tag:
	@bash scripts/release.sh --tag-only

.PHONY: release
release: build
	@bash scripts/release.sh patch

.PHONY: release-minor
release-minor: build
	@bash scripts/release.sh minor

# ── Feature tests ─────────────────────────────────────────────────────────────

.PHONY: test-features
test-features:
	@bash scripts/test-features.sh

.PHONY: test-features-chat
test-features-chat:
	@bash scripts/test-features.sh --chat

# ── Dev tools ────────────────────────────────────────────────────────────────

.PHONY: test
test:
	cargo test --workspace

.PHONY: clippy
clippy:
	cargo clippy --workspace -- -D warnings

.PHONY: fmt
fmt:
	cargo fmt --all

.PHONY: ngrok
ngrok:
	@echo "Starting ngrok tunnel → https://izzie.ngrok.dev"
	ngrok start izzie

.PHONY: clean
clean:
	cargo clean
