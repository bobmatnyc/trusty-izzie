<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import type { InstallerState } from '../App.svelte'

  let { onNext, config } = $props<{
    onNext: () => void
    config: InstallerState
  }>()

  type TaskStatus = 'pending' | 'running' | 'done' | 'error'
  type Task = { label: string; status: TaskStatus; error?: string }

  let tasks = $state<Task[]>([
    { label: 'Writing configuration', status: 'pending' },
    { label: 'Installing daemon (LaunchAgent)', status: 'pending' },
    { label: 'Starting Izzie daemon', status: 'pending' },
    { label: 'Verifying connection', status: 'pending' },
  ])

  let hasError = $state(false)
  let started = $state(false)

  function setStatus(i: number, status: TaskStatus, error?: string) {
    tasks = tasks.map((t, idx) => idx === i ? { ...t, status, error } : t)
  }

  async function runInstall() {
    hasError = false
    for (const t of tasks) {
      if (t.status === 'done') continue
      const i = tasks.findIndex(x => x.label === t.label)
      setStatus(i, 'running')
      try {
        if (i === 0) {
          await invoke('write_config', { config: serializeConfig(config) })
        } else if (i === 1) {
          await invoke('install_launch_agent')
        } else if (i === 2) {
          await invoke('start_daemon')
        } else if (i === 3) {
          await invoke('verify_daemon')
        }
        setStatus(i, 'done')
      } catch (e) {
        setStatus(i, 'error', String(e))
        hasError = true
        return
      }
    }
    setTimeout(() => onNext(), 1000)
  }

  function serializeConfig(state: InstallerState) {
    return {
      llm: state.llm,
      slack: state.slack,
      google_email: state.googleEmail,
      skills: state.skills,
    }
  }

  $effect(() => {
    if (!started) {
      started = true
      runInstall()
    }
  })
</script>

<div class="step">
  <div class="content">
    <h2>Installing Izzie</h2>

    <div class="task-list">
      {#each tasks as task}
        <div class="task" class:done={task.status === 'done'} class:error={task.status === 'error'}>
          <div class="task-icon">
            {#if task.status === 'pending'}
              <span class="pending-dot"></span>
            {:else if task.status === 'running'}
              <div class="spinner"></div>
            {:else if task.status === 'done'}
              <span class="check">✓</span>
            {:else}
              <span class="x">✕</span>
            {/if}
          </div>
          <div class="task-body">
            <span class="task-label">{task.label}</span>
            {#if task.error}
              <span class="task-error">{task.error}</span>
            {/if}
          </div>
        </div>
      {/each}
    </div>

    {#if hasError}
      <div class="error-box">
        <p>Installation failed. Check the error above and try again.</p>
        <button onclick={runInstall}>Retry</button>
      </div>
    {/if}
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
    width: 100%;
    max-width: 440px;
    display: flex;
    flex-direction: column;
    gap: 24px;
  }
  h2 { font-size: 22px; font-weight: 600; color: #111; margin: 0; }
  .task-list { display: flex; flex-direction: column; gap: 12px; }
  .task {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 12px 16px;
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 10px;
    transition: border-color 0.15s, background 0.15s;
  }
  .task.done { background: #f0fdf4; border-color: #bbf7d0; }
  .task.error { background: #fef2f2; border-color: #fecaca; }
  .task-icon { width: 20px; height: 20px; display: flex; align-items: center; justify-content: center; flex-shrink: 0; margin-top: 1px; }
  .pending-dot {
    width: 10px; height: 10px;
    border-radius: 50%;
    background: #d1d5db;
  }
  .spinner {
    width: 16px; height: 16px;
    border: 2px solid #e5e7eb;
    border-top-color: #2563eb;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  .check { color: #10b981; font-weight: 700; font-size: 15px; }
  .x { color: #dc2626; font-weight: 700; font-size: 15px; }
  .task-body { display: flex; flex-direction: column; gap: 4px; }
  .task-label { font-size: 14px; font-weight: 500; color: #374151; }
  .task-error { font-size: 12px; color: #dc2626; }
  .error-box {
    background: #fef2f2;
    border: 1px solid #fecaca;
    border-radius: 10px;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .error-box p { color: #dc2626; font-size: 13px; margin: 0; }
  button {
    align-self: flex-start;
    padding: 8px 20px;
    background: #2563eb;
    color: white;
    border: none;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
  }
  button:hover { background: #1d4ed8; }
</style>
