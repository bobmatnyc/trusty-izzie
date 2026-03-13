<script lang="ts">
  import ChatView from './ChatView.svelte'
  import HealthView from './HealthView.svelte'
  import SkillsView from './SkillsView.svelte'
  import SettingsView from './SettingsView.svelte'
  import { invoke } from '@tauri-apps/api/core'

  type Tab = 'chat' | 'health' | 'skills' | 'settings'
  let activeTab = $state<Tab>('health')
  let daemonRunning = $state(false)

  async function checkDaemon() {
    try {
      daemonRunning = await invoke<boolean>('verify_daemon')
    } catch {
      daemonRunning = false
    }
  }

  $effect(() => {
    checkDaemon()
    const interval = setInterval(checkDaemon, 10_000)
    return () => clearInterval(interval)
  })
</script>

<div class="shell">
  <nav class="sidebar">
    <div class="brand">
      <img src="/favicon.png" width="28" height="28" alt="Izzie" />
      <span class="brand-name">Izzie</span>
    </div>

    <div class="nav-items">
      <button class="nav-item" class:active={activeTab === 'chat'} onclick={() => activeTab = 'chat'}>
        <span class="nav-icon">💬</span>
        <span class="nav-label">Chat</span>
      </button>
      <button class="nav-item" class:active={activeTab === 'health'} onclick={() => activeTab = 'health'}>
        <span class="nav-icon">❤</span>
        <span class="nav-label">Health</span>
      </button>
      <button class="nav-item" class:active={activeTab === 'skills'} onclick={() => activeTab = 'skills'}>
        <span class="nav-icon">✦</span>
        <span class="nav-label">Skills</span>
      </button>
      <button class="nav-item" class:active={activeTab === 'settings'} onclick={() => activeTab = 'settings'}>
        <span class="nav-icon">⚙</span>
        <span class="nav-label">Settings</span>
      </button>
    </div>

    <div class="sidebar-footer">
      <div class="daemon-status">
        <span class="status-dot" class:running={daemonRunning}></span>
        <span class="status-label">{daemonRunning ? 'running' : 'stopped'}</span>
      </div>
      <span class="version">v0.1.7</span>
    </div>
  </nav>

  <main class="content">
    {#if activeTab === 'chat'}
      <ChatView />
    {:else if activeTab === 'health'}
      <HealthView />
    {:else if activeTab === 'skills'}
      <SkillsView />
    {:else if activeTab === 'settings'}
      <SettingsView />
    {/if}
  </main>
</div>

<style>
  .shell {
    width: 100vw;
    height: 100vh;
    display: flex;
    overflow: hidden;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
  }

  /* Sidebar */
  .sidebar {
    width: 200px;
    flex-shrink: 0;
    background: #1a1a1a;
    display: flex;
    flex-direction: column;
    padding: 16px 0;
    border-right: 1px solid #2a2a2a;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 16px 20px;
    border-bottom: 1px solid #2a2a2a;
    margin-bottom: 8px;
  }

  .brand-name {
    font-size: 16px;
    font-weight: 600;
    color: #f5f5f5;
    letter-spacing: -0.01em;
  }

  .nav-items {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0 8px;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border: none;
    background: transparent;
    color: #9ca3af;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
    text-align: left;
    width: 100%;
    transition: background 0.12s, color 0.12s;
    border-left: 2px solid transparent;
  }

  .nav-item:hover {
    background: #232323;
    color: #d4d4d4;
  }

  .nav-item.active {
    background: #2a2a2a;
    color: #f5f5f5;
    border-left-color: #2563eb;
  }

  .nav-icon {
    font-size: 14px;
    width: 18px;
    text-align: center;
    flex-shrink: 0;
  }

  .nav-label {
    flex: 1;
  }

  .sidebar-footer {
    padding: 12px 16px 0;
    border-top: 1px solid #2a2a2a;
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 8px;
  }

  .daemon-status {
    display: flex;
    align-items: center;
    gap: 7px;
  }

  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #ef4444;
    flex-shrink: 0;
    transition: background 0.3s;
  }

  .status-dot.running {
    background: #10b981;
  }

  .status-label {
    font-size: 12px;
    color: #6b7280;
  }

  .version {
    font-size: 11px;
    color: #4b5563;
  }

  /* Main content */
  .content {
    flex: 1;
    background: #f9fafb;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
</style>
