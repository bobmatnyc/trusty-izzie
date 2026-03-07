<script lang="ts">
  let { onNext } = $props()
</script>

<div class="step">
  <div class="hero">
    <div class="icon">✦</div>
    <h1>Welcome to Izzie</h1>
    <p>Your personal AI assistant, running entirely on your Mac.<br/>This installer will set everything up in a few minutes.</p>
    <button onclick={onNext}>Get Started →</button>
  </div>
</div>

<style>
  .step {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .hero {
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
  }
  .icon {
    width: 64px; height: 64px;
    background: linear-gradient(135deg, #2563eb, #4f46e5);
    border-radius: 16px;
    display: flex; align-items: center; justify-content: center;
    color: white; font-size: 28px;
  }
  h1 { font-size: 28px; font-weight: 700; color: #111; }
  p { color: #6b7280; line-height: 1.6; }
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
