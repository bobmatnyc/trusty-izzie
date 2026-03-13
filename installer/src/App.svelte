<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import Welcome from './steps/Welcome.svelte'
  import LlmBackend from './steps/LlmBackend.svelte'
  import SlackSetup from './steps/SlackSetup.svelte'
  import GoogleOAuth from './steps/GoogleOAuth.svelte'
  import SkillsSelect from './steps/SkillsSelect.svelte'
  import Installing from './steps/Installing.svelte'
  import Done from './steps/Done.svelte'
  import AppShell from './app/AppShell.svelte'
  import type { LlmConfig } from './steps/LlmBackend.svelte'
  import type { SlackConfig } from './steps/SlackSetup.svelte'
  import type { SkillsConfig } from './steps/SkillsSelect.svelte'

  export type InstallerState = {
    llm: LlmConfig
    slack: SlackConfig
    googleEmail: string | null
    skills: SkillsConfig
  }

  let installed = $state<boolean | null>(null)
  let step = $state(0)

  let state = $state<InstallerState>({
    llm: { provider: 'openrouter', apiKey: '' },
    slack: { mode: 'skip' },
    googleEmail: null,
    skills: { enabled: [], keys: {} },
  })

  const STEP_COUNT = 7

  function next() {
    // After Installing (step 5) completes, go straight to the app shell
    if (step === 5) {
      installed = true
    } else {
      step++
    }
  }
  function back() { step-- }

  $effect(() => {
    invoke<boolean>('check_installed')
      .then(v => { installed = v })
      .catch(() => { installed = false })
  })
</script>

{#if installed === null}
  <!-- Loading: detect install state -->
  <div class="loading-screen">
    <div class="loading-spinner"></div>
  </div>
{:else if installed}
  <AppShell />
{:else}
  <main class="installer">
    <header>
      <div class="logo">
        <img src="/favicon.png" width="28" height="28" alt="Izzie" class="logo-img" />
        <span class="logo-text">Izzie</span>
      </div>
      <div class="step-indicator">
        {#each { length: STEP_COUNT } as _, i}
          <div class="dot" class:active={i === step} class:done={i < step}></div>
        {/each}
      </div>
    </header>

    {#if step === 0}
      <Welcome onNext={next} />
    {:else if step === 1}
      <LlmBackend
        onNext={next}
        onBack={back}
        onUpdate={(cfg) => (state.llm = cfg)}
      />
    {:else if step === 2}
      <SlackSetup
        onNext={next}
        onBack={back}
        onUpdate={(cfg) => (state.slack = cfg)}
      />
    {:else if step === 3}
      <GoogleOAuth
        onNext={next}
        onBack={back}
        onUpdate={(email) => (state.googleEmail = email)}
      />
    {:else if step === 4}
      <SkillsSelect
        onNext={next}
        onBack={back}
        onUpdate={(cfg) => (state.skills = cfg)}
        slack={state.slack}
      />
    {:else if step === 5}
      <Installing onNext={next} config={state} />
    {:else if step === 6}
      <Done config={state} />
    {/if}
  </main>
{/if}

<style>
  .loading-screen {
    width: 100vw;
    height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #fafafa;
  }

  .loading-spinner {
    width: 24px;
    height: 24px;
    border: 2px solid #e5e7eb;
    border-top-color: #2563eb;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .installer {
    width: 100vw;
    height: 100vh;
    display: flex;
    flex-direction: column;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    background: #fafafa;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 20px 32px;
    border-bottom: 1px solid #e5e7eb;
  }

  .logo { display: flex; align-items: center; gap: 10px; font-weight: 600; }
  .logo-img { display: block; border-radius: 6px; }

  .step-indicator { display: flex; gap: 8px; }
  .dot {
    width: 8px; height: 8px; border-radius: 50%;
    background: #e5e7eb; transition: background 0.2s;
  }
  .dot.active { background: #2563eb; }
  .dot.done { background: #10b981; }
</style>
