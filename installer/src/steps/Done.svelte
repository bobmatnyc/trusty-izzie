<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import type { InstallerState } from '../App.svelte'

  let { config } = $props<{ config: InstallerState }>()

  const slackConnected = config.slack.mode !== 'skip'

  async function close() {
    await invoke('close_window')
  }
</script>

<div class="step">
  <div class="content">
    <div class="check-circle">✓</div>
    <h2>You're all set!</h2>
    <p class="subtitle">Izzie is running in the background.</p>

    <div class="status-list">
      <div class="status-row">
        <span class="status-ok">✓</span>
        <span>Daemon running</span>
      </div>
      <div class="status-row">
        {#if config.googleEmail}
          <span class="status-ok">✓</span>
          <span>Google connected as <strong>{config.googleEmail}</strong></span>
        {:else}
          <span class="status-warn">○</span>
          <span>Google: not connected</span>
        {/if}
      </div>
      <div class="status-row">
        {#if slackConnected}
          <span class="status-ok">✓</span>
          <span>Slack connected</span>
        {:else}
          <span class="status-skip">—</span>
          <span>Slack: configure later</span>
        {/if}
      </div>
    </div>

    <div class="instructions">
      {#if slackConnected}
        <p>Open Slack and send a DM to <strong>@Izzie</strong> to start chatting.</p>
      {:else}
        <p>Run <code>trusty chat</code> in your terminal to start.</p>
      {/if}
    </div>

    <button onclick={close}>Close Installer</button>
  </div>
</div>

<style>
  .step {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 40px;
  }
  .content {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
    text-align: center;
    max-width: 400px;
  }
  .check-circle {
    width: 72px; height: 72px;
    background: #10b981;
    border-radius: 50%;
    display: flex; align-items: center; justify-content: center;
    color: white; font-size: 32px; font-weight: 700;
  }
  h2 { font-size: 24px; font-weight: 700; color: #111; margin: 0; }
  .subtitle { color: #6b7280; margin: 0; }
  .status-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 100%;
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 12px;
    padding: 16px;
  }
  .status-row {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 14px;
    color: #374151;
  }
  .status-ok { color: #10b981; font-weight: 700; font-size: 15px; }
  .status-warn { color: #f59e0b; font-weight: 700; }
  .status-skip { color: #9ca3af; font-weight: 700; }
  .instructions {
    background: #f9fafb;
    border: 1px solid #e5e7eb;
    border-radius: 10px;
    padding: 16px;
    width: 100%;
  }
  .instructions p { color: #374151; font-size: 14px; margin: 0; line-height: 1.6; }
  code {
    background: #e5e7eb;
    padding: 2px 6px;
    border-radius: 4px;
    font-family: 'SF Mono', 'Fira Code', monospace;
    font-size: 13px;
  }
  button {
    margin-top: 8px;
    padding: 12px 32px;
    background: #2563eb;
    color: white;
    border: none;
    border-radius: 8px;
    font-size: 15px;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.15s;
  }
  button:hover { background: #1d4ed8; }
</style>
