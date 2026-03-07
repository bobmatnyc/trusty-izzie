<script lang="ts">
  import Welcome from './steps/Welcome.svelte'
  import ApiKey from './steps/ApiKey.svelte'
  import Installing from './steps/Installing.svelte'
  import GoogleOAuth from './steps/GoogleOAuth.svelte'
  import Done from './steps/Done.svelte'

  let step = $state(0)
  const steps = [Welcome, ApiKey, Installing, GoogleOAuth, Done]
  const CurrentStep = $derived(steps[step])
</script>

<main class="installer">
  <header>
    <div class="logo">
      <span class="logo-mark">✦</span>
      <span class="logo-text">Izzie Installer</span>
    </div>
    <div class="step-indicator">
      {#each steps as _, i}
        <div class="dot" class:active={i === step} class:done={i < step}></div>
      {/each}
    </div>
  </header>

  <CurrentStep onNext={() => step++} onBack={() => step--} />
</main>

<style>
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
  .logo-mark {
    width: 32px; height: 32px;
    background: linear-gradient(135deg, #2563eb, #4f46e5);
    border-radius: 8px;
    display: flex; align-items: center; justify-content: center;
    color: white; font-size: 16px;
  }
  .step-indicator { display: flex; gap: 8px; }
  .dot {
    width: 8px; height: 8px; border-radius: 50%;
    background: #e5e7eb; transition: background 0.2s;
  }
  .dot.active { background: #2563eb; }
  .dot.done { background: #10b981; }
</style>
