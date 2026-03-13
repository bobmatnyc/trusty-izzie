<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'

  type Config = {
    llm_provider: string
    has_openrouter_key: boolean
    aws_region: string | null
    slack_mode: string
    google_email: string | null
    data_dir: string
    skills_enabled: string[]
  }

  let config = $state<Config | null>(null)
  let loadError = $state<string | null>(null)
  let confirmReset = $state(false)
  let resetDone = $state(false)

  $effect(() => {
    invoke<Config>('read_config')
      .then(c => { config = c })
      .catch((e: unknown) => { loadError = String(e) })
  })

  async function openDataDir() {
    if (!config) return
    await invoke('open_in_finder', { path: config.data_dir })
  }

  async function reconnectGoogle() {
    await invoke('start_google_oauth')
  }

  async function doReset() {
    await invoke('reset_config')
    confirmReset = false
    resetDone = true
    config = null
  }

  function maskKey(has: boolean): string {
    return has ? '••••••••••••••••' : 'Not configured'
  }
</script>

<div class="settings-view">
  <div class="view-header">
    <h2>Settings</h2>
  </div>

  {#if loadError}
    <div class="error-banner">{loadError}</div>
  {/if}

  {#if resetDone}
    <div class="info-banner">Configuration reset. Restart Izzie to run the setup wizard again.</div>
  {/if}

  {#if config}

    <!-- LLM Backend -->
    <section>
      <div class="section-title">LLM Backend</div>
      <div class="settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">Provider</span>
            <span class="setting-value capitalize">{config.llm_provider}</span>
          </div>
        </div>
        {#if config.llm_provider === 'openrouter'}
          <div class="setting-row">
            <div class="setting-info">
              <span class="setting-label">API Key</span>
              <span class="setting-value mono">{maskKey(config.has_openrouter_key)}</span>
            </div>
          </div>
        {:else if config.llm_provider === 'bedrock'}
          <div class="setting-row">
            <div class="setting-info">
              <span class="setting-label">AWS Region</span>
              <span class="setting-value mono">{config.aws_region ?? '—'}</span>
            </div>
          </div>
        {/if}
      </div>
    </section>

    <!-- Slack -->
    <section>
      <div class="section-title">Slack</div>
      <div class="settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">Mode</span>
            <span class="setting-value capitalize">{config.slack_mode}</span>
          </div>
        </div>
      </div>
    </section>

    <!-- Google Account -->
    <section>
      <div class="section-title">Google Account</div>
      <div class="settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">Connected as</span>
            <span class="setting-value">{config.google_email ?? 'Not connected'}</span>
          </div>
          <button class="btn-sm" onclick={reconnectGoogle}>Reconnect</button>
        </div>
      </div>
    </section>

    <!-- Data -->
    <section>
      <div class="section-title">Data</div>
      <div class="settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">Data directory</span>
            <span class="setting-value mono small">{config.data_dir}</span>
          </div>
          <button class="btn-sm" onclick={openDataDir}>Open in Finder</button>
        </div>
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">Enabled skills</span>
            <span class="setting-value">
              {config.skills_enabled.length > 0 ? config.skills_enabled.join(', ') : 'None'}
            </span>
          </div>
        </div>
      </div>
    </section>

    <!-- Danger Zone -->
    <section>
      <div class="section-title danger-title">Danger Zone</div>
      <div class="settings-card danger-card">
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">Reset configuration</span>
            <span class="setting-sub">Deletes config.env only. Your data directory is untouched.</span>
          </div>
          {#if confirmReset}
            <div class="confirm-group">
              <span class="confirm-label">Are you sure?</span>
              <button class="btn-danger" onclick={doReset}>Yes, reset</button>
              <button class="btn-sm" onclick={() => confirmReset = false}>Cancel</button>
            </div>
          {:else}
            <button class="btn-danger" onclick={() => confirmReset = true}>Reset configuration</button>
          {/if}
        </div>
      </div>
    </section>

  {:else if !loadError && !resetDone}
    <div class="loading">Loading configuration…</div>
  {/if}
</div>

<style>
  .settings-view {
    padding: 28px 32px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    overflow-y: auto;
    height: 100%;
  }

  .view-header { display: flex; align-items: center; justify-content: space-between; }
  h2 { font-size: 20px; font-weight: 600; color: #111827; margin: 0; }

  .loading { color: #9ca3af; font-size: 14px; text-align: center; padding: 24px; }

  .error-banner {
    background: #fef2f2; border: 1px solid #fecaca;
    color: #dc2626; border-radius: 8px; padding: 10px 14px; font-size: 13px;
  }

  .info-banner {
    background: #eff6ff; border: 1px solid #bfdbfe;
    color: #1d4ed8; border-radius: 8px; padding: 10px 14px; font-size: 13px;
  }

  section { display: flex; flex-direction: column; gap: 8px; }

  .section-title {
    font-size: 11px; font-weight: 600; text-transform: uppercase;
    letter-spacing: 0.08em; color: #374151;
  }

  .danger-title { color: #dc2626; }

  .settings-card {
    background: white; border: 1px solid #e5e7eb;
    border-radius: 12px; overflow: hidden;
  }

  .danger-card { border-color: #fecaca; }

  .setting-row {
    display: flex; align-items: center; justify-content: space-between;
    padding: 14px 18px; gap: 16px;
    border-bottom: 1px solid #f3f4f6;
  }

  .setting-row:last-child { border-bottom: none; }

  .setting-info {
    display: flex; flex-direction: column; gap: 2px; flex: 1; min-width: 0;
  }

  .setting-label {
    font-size: 12px; font-weight: 500; color: #6b7280;
    text-transform: uppercase; letter-spacing: 0.04em;
  }

  .setting-value {
    font-size: 14px; color: #111827; word-break: break-all;
  }

  .setting-value.capitalize { text-transform: capitalize; }
  .setting-value.mono { font-family: 'SF Mono', 'Fira Code', monospace; font-size: 13px; }
  .setting-value.small { font-size: 12px; }

  .setting-sub { font-size: 12px; color: #9ca3af; margin-top: 2px; }

  .confirm-group {
    display: flex; align-items: center; gap: 8px; flex-shrink: 0;
  }

  .confirm-label { font-size: 13px; color: #374151; white-space: nowrap; }

  /* Buttons */
  .btn-sm {
    padding: 6px 14px; border: 1px solid #e5e7eb; background: white;
    border-radius: 7px; font-size: 13px; cursor: pointer; color: #374151;
    transition: background 0.12s; white-space: nowrap; flex-shrink: 0;
  }

  .btn-sm:hover { background: #f3f4f6; }

  .btn-danger {
    padding: 6px 14px; background: #fef2f2; color: #dc2626;
    border: 1px solid #fecaca; border-radius: 7px; font-size: 13px;
    font-weight: 500; cursor: pointer; transition: background 0.12s;
    white-space: nowrap; flex-shrink: 0;
  }

  .btn-danger:hover { background: #fee2e2; }
</style>
