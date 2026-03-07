# Deployment

## Overview

trusty-izzie runs as three persistent background services on macOS, managed by launchd. All services run under the current user account (LaunchAgents, not LaunchDaemons) and restart automatically on crash.

| Service | Binary | Purpose |
|---------|--------|---------|
| `com.trusty-izzie.daemon` | `trusty-daemon` | Core daemon — email sync, event queue, IPC socket |
| `com.trusty-izzie.api` | `trusty-api` | Axum REST API server on port 3456 |
| `com.trusty-izzie.telegram` | `trusty-telegram` | Telegram bot webhook receiver |

---

## Directory Layout

```
~/.local/bin/
    trusty-daemon           # Background daemon binary
    trusty-api              # REST API binary
    trusty-telegram         # Telegram bot binary
    trusty-cli              # CLI binary (also trusty symlink)

~/.local/share/trusty-izzie/
    .env                    # Runtime secrets (not committed to repo)
    instance.json           # Instance metadata
    lance/                  # LanceDB vector store
    kuzu/                   # Kuzu graph database
    trusty.db               # SQLite (auth tokens, sessions, config)
    logs/
        daemon.log          # Daemon stdout
        daemon.err.log      # Daemon stderr
        api.log             # API server stdout
        api.err.log         # API server stderr
        telegram.log        # Telegram bot stdout
        telegram.err.log    # Telegram bot stderr

~/Library/LaunchAgents/
    com.trusty-izzie.daemon.plist
    com.trusty-izzie.api.plist
    com.trusty-izzie.telegram.plist
```

---

## launchd Service Architecture

### Plist Templates

Plist templates live in `launchd/` at the repository root and are checked into version control. They use `__HOME__` as a placeholder for the user's home directory. The install script substitutes the real path at install time.

Template location: `launchd/com.trusty-izzie.{daemon,api,telegram}.plist`
Install destination: `~/Library/LaunchAgents/com.trusty-izzie.{daemon,api,telegram}.plist`

### Service Configuration

All three services share these settings:

```xml
<key>RunAtLoad</key>
<true/>

<key>KeepAlive</key>
<dict>
    <key>SuccessfulExit</key>
    <false/>
</dict>

<key>ThrottleInterval</key>
<integer>10</integer>
```

- `RunAtLoad: true` — service starts immediately when loaded (and on login)
- `KeepAlive.SuccessfulExit: false` — restart on crash, but not on clean exit (allows `trusty-daemon stop` to cleanly stop the service)
- `ThrottleInterval: 10` — minimum 10 seconds between restart attempts

### Environment Variables

Each plist includes:

```xml
<key>EnvironmentVariables</key>
<dict>
    <key>HOME</key>
    <string>/Users/username</string>
    <key>PATH</key>
    <string>/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>
    <key>TRUSTY_DATA_DIR</key>
    <string>/Users/username/.local/share/trusty-izzie</string>
</dict>
```

Runtime secrets (`OPENROUTER_API_KEY`, `GOOGLE_CLIENT_ID`, etc.) are loaded by each binary at startup from `$TRUSTY_DATA_DIR/.env`. This avoids embedding secrets in plist files (which appear in `launchctl list` output) and matches the pattern used in development.

---

## Install Process

### One-Command Install

```bash
make install
```

Or directly:

```bash
./scripts/install.sh
```

The install script:
1. Runs `cargo build --release` to build all binaries
2. Creates `~/.local/bin/`, `~/.local/share/trusty-izzie/logs/`, and `~/Library/LaunchAgents/`
3. Copies release binaries to `~/.local/bin/`
4. For each plist template in `launchd/`:
   - Substitutes `__HOME__` with `$HOME` using `sed`
   - Writes the resolved plist to `~/Library/LaunchAgents/`
   - Unloads any existing version (`launchctl unload`, ignoring errors)
   - Loads the new version (`launchctl load`)
5. Waits 2 seconds and prints service status

### Prerequisites

- macOS (tested on macOS 14+)
- Rust toolchain installed (`rustup`)
- `.env` secrets file present at project root or `~/.local/share/trusty-izzie/.env`
- Google OAuth credentials configured (see `docs/specs/06-email-pipeline.md`)

---

## Uninstall Process

```bash
make uninstall
```

Or directly:

```bash
./scripts/uninstall.sh
```

The uninstall script:
1. Unloads and removes all `com.trusty-izzie.*.plist` files from `~/Library/LaunchAgents/`
2. Prompts before removing binaries from `~/.local/bin/`
3. Prompts before wiping data — but does NOT auto-delete data (requires manual `rm -rf`)

The data directory is never automatically deleted to prevent accidental loss of the local vector store and graph database.

---

## Viewing Logs

```bash
# Daemon logs (real-time)
tail -f ~/.local/share/trusty-izzie/logs/daemon.log
tail -f ~/.local/share/trusty-izzie/logs/daemon.err.log

# API server logs
tail -f ~/.local/share/trusty-izzie/logs/api.log

# Telegram bot logs
tail -f ~/.local/share/trusty-izzie/logs/telegram.log

# All logs together
tail -f ~/.local/share/trusty-izzie/logs/*.log

# Makefile shortcuts
make logs          # daemon.log (via /tmp/trusty-daemon.log symlink)
make api-logs      # api.log (via /tmp/trusty-api.log symlink)
```

---

## Service Status

```bash
# Check all trusty-izzie services
make launchd-status

# Raw launchctl output
launchctl list | grep trusty-izzie

# Output format: PID, Exit, Label
#   12345  0  com.trusty-izzie.daemon
#   -      0  com.trusty-izzie.api      ← PID "-" means not running
```

A PID of `-` with exit code `0` means the service exited cleanly (no restart). A PID of `-` with a non-zero exit code means the service crashed — check `*.err.log`.

---

## Environment Variable Management

Secrets are stored in `~/.local/share/trusty-izzie/.env` (not in the repo). Format:

```bash
OPENROUTER_API_KEY=sk-or-v1-...
GOOGLE_CLIENT_ID=409456389838-...
GOOGLE_CLIENT_SECRET=...
TRUSTY_PRIMARY_EMAIL=your@gmail.com
TELEGRAM_BOT_TOKEN=...
```

Each binary reads this file at startup using the `dotenvy` crate (or equivalent). The plist sets `TRUSTY_DATA_DIR` so the binary knows where to find `.env`.

**Security**: The `.env` file should be mode `600` (readable only by the owner):
```bash
chmod 600 ~/.local/share/trusty-izzie/.env
```

---

## Development vs Production

### Development Mode

Run services directly in the foreground for live output and fast iteration:

```bash
# Start individual services
make run-dev        # daemon (foreground, dev build)
make api            # API server (foreground)
make telegram       # Telegram bot (foreground)

# Or via cargo
cargo run --bin trusty-daemon -- start --foreground
cargo run --bin trusty-api
```

### Production Mode (launchd)

Install once; services auto-start on login and restart on crash:

```bash
make install        # Build + install + load services
make launchd-status # Verify services are running
```

After install, services persist across reboots. Re-run `make install` after building new versions.

### Switching Between Modes

Stop launchd services before running in development mode to avoid port conflicts:

```bash
# Stop launchd services temporarily
launchctl unload ~/Library/LaunchAgents/com.trusty-izzie.daemon.plist
launchctl unload ~/Library/LaunchAgents/com.trusty-izzie.api.plist

# Run in dev mode
make run-dev
make api

# Reload launchd services when done
launchctl load ~/Library/LaunchAgents/com.trusty-izzie.daemon.plist
launchctl load ~/Library/LaunchAgents/com.trusty-izzie.api.plist
```

---

## ngrok Tunnel Setup

The Telegram webhook requires a public HTTPS URL. ngrok provides this tunnel.

### Configuration

ngrok domain is fixed: `izzie.ngrok.dev` → `localhost:3456`

The ngrok config (`~/.config/ngrok/ngrok.yml`) should contain:

```yaml
tunnels:
  izzie:
    proto: http
    addr: 3456
    domain: izzie.ngrok.dev
```

### Starting the Tunnel

```bash
# Via Makefile
make ngrok

# Directly
ngrok start izzie
```

The tunnel must be running for Telegram webhooks to reach the local API server. The Telegram plist hardcodes the webhook URL as `https://izzie.ngrok.dev/webhook/telegram`.

### Boot Persistence

ngrok is not managed by launchd in this setup. Start it manually or add a fourth plist for `ngrok start izzie` following the same pattern as the other plists.

---

## Troubleshooting

### Service fails to start

```bash
# Check error log
cat ~/.local/share/trusty-izzie/logs/daemon.err.log

# Check launchctl for exit status
launchctl list | grep trusty-izzie

# Try running binary directly to see output
~/.local/bin/trusty-daemon start --foreground
```

### Port already in use (3456)

```bash
lsof -i :3456
# Kill the conflicting process or stop the dev server
```

### Binary not found after install

```bash
# Verify binary exists
ls -la ~/.local/bin/trusty-*

# Verify PATH includes ~/.local/bin
echo $PATH | grep -o '[^:]*\.local/bin[^:]*'
```

### Plist substitution not applied

If `__HOME__` appears literally in an installed plist, the `sed` substitution failed. Check the installed plist:

```bash
cat ~/Library/LaunchAgents/com.trusty-izzie.daemon.plist | grep -c __HOME__
# Should output: 0
```

Re-run `make install` to fix.
