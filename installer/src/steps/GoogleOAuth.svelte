<script lang="ts">
  let { onNext, onBack } = $props()
</script>

<div class="step">
  <h2>Connect Google Account</h2>
  <p>Optionally connect your Google account for calendar and email access.</p>
  <div class="actions">
    <button class="secondary" onclick={onBack}>← Back</button>
    <button onclick={onNext}>Continue →</button>
  </div>
</div>

<style>
  .step {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 16px;
    padding: 40px;
  }
  h2 { font-size: 22px; font-weight: 600; color: #111; }
  p { color: #6b7280; }
  .actions { display: flex; gap: 12px; margin-top: 8px; }
  button {
    padding: 10px 28px;
    background: #2563eb;
    color: white;
    border: none;
    border-radius: 8px;
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
  }
  button.secondary {
    background: #f3f4f6;
    color: #374151;
  }
</style>
