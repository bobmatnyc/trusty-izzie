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
  }

  let health = $state<HealthData | null>(null)
  let loading = $state(true)
  let error = $state<string | null>(null)
  let actionPending = $state(false)

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
      }
    } finally {
      loading = false
    }
  }

  async function startDaemon() {
    actionPending = true
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

  $effect(() => { refresh() })
</script>

<div class="health">
  <div class="view-header">
    <h2>Health</h2>
    <button class="btn-sm" onclick={refresh} disabled={loading}>
      {loading ? 'Refreshing…' : 'Refresh'}
    </button>
  </div>

  {#if error}
    <div class="error-banner">{error}</div>
  {/if}

  <!-- Daemon status card -->
  <div class="status-card" class:running={health?.daemon}>
    <div class="status-left">
      <div class="big-dot" class:running={health?.daemon}></div>
      <div class="status-info">
        <span class="status-title">{health?.daemon ? 'Daemon running' : 'Daemon stopped'}</span>
        {#if health?.version}
          <span class="status-sub">v{health.version}</span>
        {:else}
          <span class="status-sub">trusty-daemon</span>
        {/if}
      </div>
    </div>
    <div class="status-actions">
      {#if health?.daemon}
        <button class="btn-danger" onclick={stopDaemon} disabled={actionPending}>Stop</button>
      {:else}
        <button class="btn-primary" onclick={startDaemon} disabled={actionPending}>Start</button>
      {/if}
    </div>
  </div>

  <!-- Stats grid -->
  <div class="stats-grid">
    <div class="stat-card">
      <span class="stat-value">
        {loading ? '—' : (health?.entityCount != null ? health.entityCount.toLocaleString() : '—')}
      </span>
      <span class="stat-label">Entities</span>
    </div>
    <div class="stat-card">
      <span class="stat-value">
        {loading ? '—' : (health?.memoryCount != null ? health.memoryCount.toLocaleString() : '—')}
      </span>
      <span class="stat-label">Memories</span>
    </div>
    <div class="stat-card">
      <span class="stat-value">
        {loading ? '—' : (health?.lastEmailSync ?? '—')}
      </span>
      <span class="stat-label">Last Sync</span>
    </div>
    <div class="stat-card">
      <span class="stat-value">
        {loading ? '—' : (health?.daemonUptime ?? '—')}
      </span>
      <span class="stat-label">Uptime</span>
    </div>
  </div>
</div>

<style>
  .health {
    padding: 28px 32px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    overflow-y: auto;
    height: 100%;
  }

  .view-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  h2 {
    font-size: 20px;
    font-weight: 600;
    color: #111827;
    margin: 0;
  }

  .error-banner {
    background: #fef2f2;
    border: 1px solid #fecaca;
    color: #dc2626;
    border-radius: 8px;
    padding: 10px 14px;
    font-size: 13px;
  }

  .status-card {
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 12px;
    padding: 20px 24px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }

  .status-card.running {
    border-color: #a7f3d0;
    background: #f0fdf4;
  }

  .status-left {
    display: flex;
    align-items: center;
    gap: 16px;
  }

  .big-dot {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: #ef4444;
    flex-shrink: 0;
    transition: background 0.3s;
  }

  .big-dot.running {
    background: #10b981;
    box-shadow: 0 0 0 4px rgba(16, 185, 129, 0.15);
  }

  .status-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .status-title {
    font-size: 15px;
    font-weight: 600;
    color: #111827;
  }

  .status-sub {
    font-size: 12px;
    color: #6b7280;
  }

  .status-actions {
    display: flex;
    gap: 8px;
  }

  .stats-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  .stat-card {
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 12px;
    padding: 20px 24px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .stat-value {
    font-size: 28px;
    font-weight: 700;
    color: #111827;
    font-variant-numeric: tabular-nums;
  }

  .stat-label {
    font-size: 12px;
    color: #6b7280;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-weight: 500;
  }

  /* Buttons */
  .btn-sm {
    padding: 6px 14px;
    border: 1px solid #e5e7eb;
    background: white;
    border-radius: 7px;
    font-size: 13px;
    cursor: pointer;
    color: #374151;
    transition: background 0.12s;
  }

  .btn-sm:hover:not(:disabled) { background: #f3f4f6; }
  .btn-sm:disabled { opacity: 0.5; cursor: not-allowed; }

  .btn-primary {
    padding: 8px 20px;
    background: #2563eb;
    color: white;
    border: none;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.12s;
  }

  .btn-primary:hover:not(:disabled) { background: #1d4ed8; }
  .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }

  .btn-danger {
    padding: 8px 20px;
    background: #fef2f2;
    color: #dc2626;
    border: 1px solid #fecaca;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.12s;
  }

  .btn-danger:hover:not(:disabled) { background: #fee2e2; }
  .btn-danger:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
