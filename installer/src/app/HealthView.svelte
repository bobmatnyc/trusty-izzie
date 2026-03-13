<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'

  type HealthData = {
    daemon: boolean
    apiReachable: boolean
    entityCount: number | null
    memoryCount: number | null
    lastEmailSync: string | null
    daemonUptime: string | null
    version: string | null
    // Extended fields (populated when daemon is running)
    emailsProcessed: number | null
    pendingActions: number | null
    slackConnected: boolean | null
    chatSessions: number | null
    lastActivity: string | null
  }

  let health = $state<HealthData | null>(null)
  let loading = $state(true)
  let error = $state<string | null>(null)
  let actionPending = $state(false)
  let lastRefreshed = $state<Date | null>(null)
  let countdown = $state(30)

  async function refresh() {
    loading = true
    error = null
    try {
      const res = await fetch('http://localhost:3456/health', {
        signal: AbortSignal.timeout(3000),
      })
      const data = await res.json()
      health = {
        daemon: true,
        apiReachable: true,
        entityCount: data.entity_count ?? null,
        memoryCount: data.memory_count ?? null,
        lastEmailSync: data.last_email_sync ?? null,
        daemonUptime: data.uptime ?? null,
        version: data.version ?? null,
        emailsProcessed: data.emails_processed ?? null,
        pendingActions: data.pending_actions ?? null,
        slackConnected: data.slack_connected ?? null,
        chatSessions: data.chat_sessions ?? null,
        lastActivity: data.last_activity ?? null,
      }
    } catch {
      health = {
        daemon: false,
        apiReachable: false,
        entityCount: null,
        memoryCount: null,
        lastEmailSync: null,
        daemonUptime: null,
        version: null,
        emailsProcessed: null,
        pendingActions: null,
        slackConnected: null,
        chatSessions: null,
        lastActivity: null,
      }
    } finally {
      loading = false
      lastRefreshed = new Date()
      countdown = 30
    }
  }

  async function startDaemon() {
    actionPending = true
    error = null
    try {
      await invoke('start_daemon')
      await new Promise(r => setTimeout(r, 1500))
      await refresh()
    } catch (e) {
      error = String(e)
    } finally {
      actionPending = false
    }
  }

  async function stopDaemon() {
    actionPending = true
    error = null
    try {
      await invoke('stop_daemon')
      await new Promise(r => setTimeout(r, 800))
      await refresh()
    } catch (e) {
      error = String(e)
    } finally {
      actionPending = false
    }
  }

  function fmt(n: number | null | undefined): string {
    if (n == null) return '—'
    return n.toLocaleString()
  }

  function timeAgo(dateStr: string | null | undefined): string {
    if (!dateStr) return '—'
    try {
      const d = new Date(dateStr)
      const diff = Math.floor((Date.now() - d.getTime()) / 1000)
      if (diff < 60) return `${diff}s ago`
      if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
      if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`
      return `${Math.floor(diff / 86400)}d ago`
    } catch {
      return dateStr
    }
  }

  // Auto-refresh every 30 seconds with visible countdown
  $effect(() => {
    refresh()
    const tick = setInterval(() => {
      countdown = Math.max(0, countdown - 1)
      if (countdown === 0) {
        refresh()
      }
    }, 1000)
    return () => clearInterval(tick)
  })
</script>

<div class="health">
  <div class="view-header">
    <div class="header-left">
      <h2>Status</h2>
      {#if lastRefreshed}
        <span class="last-refresh">Updated {timeAgo(lastRefreshed.toISOString())} · refreshing in {countdown}s</span>
      {/if}
    </div>
    <button class="btn-sm" onclick={refresh} disabled={loading}>
      {#if loading}
        <span class="spin">↻</span> Refreshing…
      {:else}
        ↻ Refresh
      {/if}
    </button>
  </div>

  {#if error}
    <div class="error-banner">⚠ {error}</div>
  {/if}

  <!-- Hero status card -->
  <div class="hero-card" class:running={health?.daemon} class:stopped={health != null && !health.daemon}>
    <div class="hero-left">
      <div class="pulse-ring" class:active={health?.daemon}>
        <div class="pulse-dot" class:active={health?.daemon}></div>
      </div>
      <div class="hero-text">
        <div class="hero-title">
          {#if loading && !health}
            Checking…
          {:else if health?.daemon}
            Daemon running
          {:else}
            Daemon stopped
          {/if}
        </div>
        <div class="hero-sub">
          {#if health?.version}trusty-daemon v{health.version}{:else}trusty-daemon{/if}
          {#if health?.daemonUptime} · up {health.daemonUptime}{/if}
        </div>
      </div>
    </div>
    <div class="hero-actions">
      {#if health?.daemon}
        <button class="btn-stop" onclick={stopDaemon} disabled={actionPending}>
          {actionPending ? '…' : 'Stop'}
        </button>
      {:else}
        <button class="btn-start" onclick={startDaemon} disabled={actionPending}>
          {actionPending ? 'Starting…' : 'Start Daemon'}
        </button>
      {/if}
    </div>
  </div>

  <!-- Primary metrics row -->
  <div class="metrics-row">
    <div class="metric-card">
      <div class="metric-icon entity-icon">◈</div>
      <div class="metric-body">
        <div class="metric-value">{fmt(health?.entityCount)}</div>
        <div class="metric-label">People &amp; Orgs</div>
      </div>
    </div>
    <div class="metric-card">
      <div class="metric-icon memory-icon">◆</div>
      <div class="metric-body">
        <div class="metric-value">{fmt(health?.memoryCount)}</div>
        <div class="metric-label">Memories</div>
      </div>
    </div>
    <div class="metric-card">
      <div class="metric-icon email-icon">✉</div>
      <div class="metric-body">
        <div class="metric-value">{fmt(health?.emailsProcessed)}</div>
        <div class="metric-label">Emails Indexed</div>
      </div>
    </div>
    <div class="metric-card">
      <div class="metric-icon chat-icon">💬</div>
      <div class="metric-body">
        <div class="metric-value">{fmt(health?.chatSessions)}</div>
        <div class="metric-label">Chat Sessions</div>
      </div>
    </div>
  </div>

  <!-- Secondary status row -->
  <div class="status-row">
    <div class="status-item">
      <div class="status-key">Last email sync</div>
      <div class="status-val">{timeAgo(health?.lastEmailSync)}</div>
    </div>
    <div class="status-divider"></div>
    <div class="status-item">
      <div class="status-key">Slack</div>
      <div class="status-val" class:status-online={health?.slackConnected === true} class:status-offline={health?.slackConnected === false}>
        {#if health?.slackConnected === true}
          <span class="inline-dot online"></span> Connected
        {:else if health?.slackConnected === false}
          <span class="inline-dot offline"></span> Disconnected
        {:else}
          —
        {/if}
      </div>
    </div>
    <div class="status-divider"></div>
    <div class="status-item">
      <div class="status-key">Pending actions</div>
      <div class="status-val" class:status-pending={health != null && (health.pendingActions ?? 0) > 0}>
        {health?.pendingActions != null ? health.pendingActions : '—'}
        {#if health?.pendingActions != null && health.pendingActions > 0}
          <span class="badge">{health.pendingActions}</span>
        {/if}
      </div>
    </div>
    <div class="status-divider"></div>
    <div class="status-item">
      <div class="status-key">Last activity</div>
      <div class="status-val">{timeAgo(health?.lastActivity)}</div>
    </div>
  </div>

  <!-- Connection checks -->
  <div class="connections-section">
    <div class="section-title">Connections</div>
    <div class="connections-grid">
      <div class="conn-row">
        <div class="conn-dot" class:ok={health?.daemon} class:err={health != null && !health.daemon}></div>
        <span class="conn-name">Daemon API</span>
        <span class="conn-addr">localhost:3456</span>
        <span class="conn-status">{health?.daemon ? 'reachable' : (health ? 'unreachable' : '…')}</span>
      </div>
      <div class="conn-row">
        <div class="conn-dot" class:ok={health?.slackConnected === true} class:err={health?.slackConnected === false}></div>
        <span class="conn-name">Slack</span>
        <span class="conn-addr">socket mode</span>
        <span class="conn-status">{health?.slackConnected === true ? 'connected' : health?.slackConnected === false ? 'not connected' : '—'}</span>
      </div>
      <div class="conn-row">
        <div class="conn-dot" class:ok={health?.lastEmailSync != null}></div>
        <span class="conn-name">Gmail</span>
        <span class="conn-addr">SENT only</span>
        <span class="conn-status">{health?.lastEmailSync ? `synced ${timeAgo(health.lastEmailSync)}` : '—'}</span>
      </div>
    </div>
  </div>
</div>

<style>
  .health {
    padding: 24px 28px;
    display: flex;
    flex-direction: column;
    gap: 16px;
    overflow-y: auto;
    height: 100%;
    box-sizing: border-box;
  }

  /* Header */
  .view-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-shrink: 0;
  }

  .header-left {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }

  h2 {
    font-size: 20px;
    font-weight: 700;
    color: #111827;
    margin: 0;
    letter-spacing: -0.02em;
  }

  .last-refresh {
    font-size: 11px;
    color: #9ca3af;
  }

  .error-banner {
    background: #fef2f2;
    border: 1px solid #fecaca;
    color: #dc2626;
    border-radius: 10px;
    padding: 10px 14px;
    font-size: 13px;
    flex-shrink: 0;
  }

  /* Hero card */
  .hero-card {
    background: white;
    border: 1.5px solid #e5e7eb;
    border-radius: 14px;
    padding: 22px 24px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    flex-shrink: 0;
    transition: border-color 0.3s, background 0.3s;
  }

  .hero-card.running {
    border-color: #6ee7b7;
    background: linear-gradient(135deg, #f0fdf4 0%, #ecfdf5 100%);
  }

  .hero-card.stopped {
    border-color: #fecaca;
    background: linear-gradient(135deg, #fff5f5 0%, #fef2f2 100%);
  }

  .hero-left {
    display: flex;
    align-items: center;
    gap: 18px;
  }

  /* Animated pulse ring */
  .pulse-ring {
    width: 40px;
    height: 40px;
    border-radius: 50%;
    background: #f3f4f6;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    position: relative;
  }

  .pulse-ring.active {
    background: rgba(16, 185, 129, 0.12);
  }

  .pulse-ring.active::after {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: 50%;
    border: 2px solid rgba(16, 185, 129, 0.4);
    animation: pulse-ring 2s ease-out infinite;
  }

  @keyframes pulse-ring {
    0% { transform: scale(1); opacity: 0.6; }
    100% { transform: scale(1.5); opacity: 0; }
  }

  .pulse-dot {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #d1d5db;
    transition: background 0.3s;
    position: relative;
    z-index: 1;
  }

  .pulse-dot.active {
    background: #10b981;
  }

  .hero-text {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .hero-title {
    font-size: 16px;
    font-weight: 700;
    color: #111827;
    letter-spacing: -0.01em;
  }

  .hero-sub {
    font-size: 12px;
    color: #6b7280;
  }

  /* Primary metrics */
  .metrics-row {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 10px;
    flex-shrink: 0;
  }

  .metric-card {
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 12px;
    padding: 16px 16px 14px;
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }

  .metric-icon {
    font-size: 18px;
    line-height: 1;
    margin-top: 2px;
    flex-shrink: 0;
  }

  .entity-icon { color: #6366f1; }
  .memory-icon { color: #8b5cf6; }
  .email-icon  { color: #0ea5e9; }
  .chat-icon   { color: #10b981; }

  .metric-body {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .metric-value {
    font-size: 22px;
    font-weight: 700;
    color: #111827;
    letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
  }

  .metric-label {
    font-size: 11px;
    color: #9ca3af;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-weight: 500;
  }

  /* Secondary status bar */
  .status-row {
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 12px;
    padding: 14px 20px;
    display: flex;
    align-items: center;
    gap: 0;
    flex-shrink: 0;
  }

  .status-item {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .status-divider {
    width: 1px;
    height: 32px;
    background: #e5e7eb;
    margin: 0 16px;
    flex-shrink: 0;
  }

  .status-key {
    font-size: 11px;
    color: #9ca3af;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-weight: 500;
  }

  .status-val {
    font-size: 13px;
    font-weight: 600;
    color: #374151;
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .status-pending { color: #d97706; }

  .badge {
    background: #fef3c7;
    color: #d97706;
    border-radius: 999px;
    padding: 1px 6px;
    font-size: 10px;
    font-weight: 700;
  }

  .inline-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    display: inline-block;
    flex-shrink: 0;
  }

  .inline-dot.online  { background: #10b981; }
  .inline-dot.offline { background: #ef4444; }

  /* Connections section */
  .connections-section {
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 12px;
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .section-title {
    font-size: 11px;
    font-weight: 600;
    color: #9ca3af;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .connections-grid {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .conn-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 0;
    border-bottom: 1px solid #f3f4f6;
  }

  .conn-row:last-child { border-bottom: none; }

  .conn-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #d1d5db;
    flex-shrink: 0;
  }

  .conn-dot.ok  { background: #10b981; }
  .conn-dot.err { background: #ef4444; }

  .conn-name {
    font-size: 13px;
    font-weight: 500;
    color: #374151;
    width: 100px;
    flex-shrink: 0;
  }

  .conn-addr {
    font-size: 12px;
    color: #9ca3af;
    font-family: 'SF Mono', 'Fira Mono', monospace;
    flex: 1;
  }

  .conn-status {
    font-size: 12px;
    color: #6b7280;
  }

  /* Buttons */
  .btn-sm {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 6px 12px;
    border: 1px solid #e5e7eb;
    background: white;
    border-radius: 7px;
    font-size: 12px;
    cursor: pointer;
    color: #6b7280;
    transition: background 0.12s, color 0.12s;
  }

  .btn-sm:hover:not(:disabled) { background: #f3f4f6; color: #374151; }
  .btn-sm:disabled { opacity: 0.5; cursor: not-allowed; }

  .spin {
    display: inline-block;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .btn-start {
    padding: 9px 22px;
    background: #2563eb;
    color: white;
    border: none;
    border-radius: 9px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.12s;
    letter-spacing: -0.01em;
  }

  .btn-start:hover:not(:disabled) { background: #1d4ed8; }
  .btn-start:disabled { opacity: 0.5; cursor: not-allowed; }

  .btn-stop {
    padding: 9px 22px;
    background: white;
    color: #dc2626;
    border: 1.5px solid #fecaca;
    border-radius: 9px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.12s;
  }

  .btn-stop:hover:not(:disabled) { background: #fef2f2; }
  .btn-stop:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
