<script lang="ts">
  let { onBack } = $props()
</script>

<div class="step">
  <div class="hero">
    <div class="icon">✓</div>
    <h2>Izzie is Ready</h2>
    <p>Setup complete. Izzie is running in the background.</p>
    <button onclick={() => window.close()}>Launch Izzie</button>
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
    background: #10b981;
    border-radius: 50%;
    display: flex; align-items: center; justify-content: center;
    color: white; font-size: 28px; font-weight: 700;
  }
  h2 { font-size: 22px; font-weight: 600; color: #111; }
  p { color: #6b7280; }
  button {
    margin-top: 8px;
    padding: 12px 32px;
    background: #10b981;
    color: white;
    border: none;
    border-radius: 8px;
    font-size: 15px;
    font-weight: 500;
    cursor: pointer;
  }
</style>
