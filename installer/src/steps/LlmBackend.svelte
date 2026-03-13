<script lang="ts">
  export type LlmConfig =
    | { provider: 'openrouter'; apiKey: string }
    | { provider: 'bedrock'; region: string; modelId: string }

  let { onNext, onBack, onUpdate } = $props<{
    onNext: () => void
    onBack: () => void
    onUpdate: (cfg: LlmConfig) => void
  }>()

  let provider = $state<'openrouter' | 'bedrock'>('openrouter')
  let apiKey = $state('')
  let region = $state('us-east-1')
  let modelId = $state('anthropic.claude-sonnet-4-5-20251001-v1:0')

  function canContinue() {
    if (provider === 'openrouter') return apiKey.trim().length > 0
    return region.trim().length > 0 && modelId.trim().length > 0
  }

  function handleNext() {
    const cfg: LlmConfig =
      provider === 'openrouter'
        ? { provider: 'openrouter', apiKey: apiKey.trim() }
        : { provider: 'bedrock', region: region.trim(), modelId: modelId.trim() }
    onUpdate(cfg)
    onNext()
  }
</script>

<div class="step">
  <div class="content">
    <h2>AI Backend</h2>
    <p class="subtitle">Choose where Izzie's AI processing runs</p>

    <div class="cards">
      <label class="card" class:selected={provider === 'openrouter'}>
        <div class="card-header">
          <input type="radio" name="provider" value="openrouter" bind:group={provider} />
          <span class="card-title">OpenRouter <span class="badge">Recommended</span></span>
        </div>
        <p class="card-desc">Access Claude, GPT-4, and more through one API</p>
        {#if provider === 'openrouter'}
          <div class="field">
            <label for="api-key">API Key</label>
            <input
              id="api-key"
              type="password"
              placeholder="sk-or-v1-_______________"
              bind:value={apiKey}
            />
            <a href="https://openrouter.ai/keys" target="_blank" class="link">Get a key at openrouter.ai →</a>
          </div>
        {/if}
      </label>

      <label class="card" class:selected={provider === 'bedrock'}>
        <div class="card-header">
          <input type="radio" name="provider" value="bedrock" bind:group={provider} />
          <span class="card-title">AWS Bedrock</span>
        </div>
        <p class="card-desc">Use your AWS account — ideal if you have Enterprise credits</p>
        {#if provider === 'bedrock'}
          <div class="fields">
            <div class="field">
              <label for="aws-region">AWS Region</label>
              <input id="aws-region" type="text" placeholder="us-east-1" bind:value={region} />
            </div>
            <div class="field">
              <label for="model-id">Model ID</label>
              <input id="model-id" type="text" placeholder="anthropic.claude-..." bind:value={modelId} />
            </div>
            <p class="note">Uses ~/.aws/credentials — no key entry needed</p>
          </div>
        {/if}
      </label>
    </div>

    <div class="actions">
      <button class="secondary" onclick={onBack}>← Back</button>
      <button onclick={handleNext} disabled={!canContinue()}>Continue →</button>
    </div>
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
    max-width: 520px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  h2 { font-size: 22px; font-weight: 600; color: #111; margin: 0; }
  .subtitle { color: #6b7280; margin: 0; }
  .cards { display: flex; flex-direction: column; gap: 12px; }
  .card {
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 12px;
    padding: 16px;
    cursor: pointer;
    transition: border-color 0.15s, background 0.15s;
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-left: 3px solid transparent;
  }
  .card.selected {
    background: #eff6ff;
    border-color: #2563eb;
    border-left-color: #2563eb;
  }
  .card-header {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .card-title { font-weight: 600; color: #111; display: flex; align-items: center; gap: 8px; }
  .badge {
    font-size: 11px;
    font-weight: 500;
    background: #dbeafe;
    color: #1d4ed8;
    padding: 2px 8px;
    border-radius: 10px;
  }
  .card-desc { color: #6b7280; font-size: 13px; margin: 0; }
  .field, .fields { display: flex; flex-direction: column; gap: 6px; margin-top: 4px; }
  .fields { gap: 10px; }
  .field label { font-size: 12px; font-weight: 500; color: #374151; }
  input[type="text"], input[type="password"] {
    border: 1px solid #d1d5db;
    border-radius: 8px;
    padding: 10px 14px;
    font-size: 13px;
    font-family: inherit;
    width: 100%;
    box-sizing: border-box;
  }
  input[type="text"]:focus, input[type="password"]:focus {
    outline: none;
    border-color: #2563eb;
    box-shadow: 0 0 0 3px rgba(37,99,235,0.1);
  }
  input[type="radio"] { accent-color: #2563eb; }
  .link { font-size: 12px; color: #2563eb; text-decoration: none; }
  .link:hover { text-decoration: underline; }
  .note { font-size: 12px; color: #9ca3af; margin: 0; font-style: italic; }
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
    transition: background 0.15s;
  }
  button:hover:not(:disabled) { background: #1d4ed8; }
  button:disabled { opacity: 0.4; cursor: not-allowed; }
  button.secondary { background: #f3f4f6; color: #374151; }
  button.secondary:hover { background: #e5e7eb; }
</style>
