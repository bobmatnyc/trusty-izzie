<script lang="ts">
  export type SlackConfig =
    | { mode: 'self'; botToken: string; appToken: string; userToken?: string; webhookUrl: string }
    | { mode: 'managed'; routerUrl: string; routerToken: string; webhookUrl: string }
    | { mode: 'skip' }

  let { onNext, onBack, onUpdate } = $props<{
    onNext: () => void
    onBack: () => void
    onUpdate: (cfg: SlackConfig) => void
  }>()

  let mode = $state<'self' | 'managed' | 'skip'>('skip')
  let botToken = $state('')
  let appToken = $state('')
  let userToken = $state('')
  let routerUrl = $state('https://router.izzie.dev')
  let routerToken = $state('')
  let webhookUrl = $state('')

  function canContinue() {
    if (mode === 'self') return botToken.trim().length > 0 && appToken.trim().length > 0
    if (mode === 'managed') return routerUrl.trim().length > 0 && routerToken.trim().length > 0
    return true
  }

  function handleNext() {
    let cfg: SlackConfig
    if (mode === 'self') {
      cfg = { mode: 'self', botToken: botToken.trim(), appToken: appToken.trim(), webhookUrl: webhookUrl.trim() }
      if (userToken.trim()) cfg = { ...cfg, userToken: userToken.trim() }
    } else if (mode === 'managed') {
      cfg = { mode: 'managed', routerUrl: routerUrl.trim(), routerToken: routerToken.trim(), webhookUrl: webhookUrl.trim() }
    } else {
      cfg = { mode: 'skip' }
    }
    onUpdate(cfg)
    onNext()
  }
</script>

<div class="step">
  <div class="content">
    <h2>Slack</h2>
    <p class="subtitle">Chat with Izzie through Slack</p>

    <div class="cards">
      <label class="card" class:selected={mode === 'self'}>
        <div class="card-header">
          <input type="radio" name="mode" value="self" bind:group={mode} />
          <span class="card-title">Self-hosted bot</span>
          <span class="card-tag">I'll create my own Slack app</span>
        </div>
        <p class="card-desc">Full control — your bot, your workspace</p>
        {#if mode === 'self'}
          <div class="fields">
            <div class="field">
              <label for="bot-token">Bot Token</label>
              <input id="bot-token" type="password" placeholder="xoxb-_______________" bind:value={botToken} />
            </div>
            <div class="field">
              <label for="app-token">App Token <span class="label-note">(Socket Mode)</span></label>
              <input id="app-token" type="password" placeholder="xapp-_______________" bind:value={appToken} />
            </div>
            <div class="field">
              <label for="user-token">User Token <span class="label-note">(optional — post as you)</span></label>
              <input id="user-token" type="password" placeholder="xoxp-_______________" bind:value={userToken} />
            </div>
            <div class="field">
              <label for="webhook-url-self">Incoming Webhook URL <span class="label-note">(optional — proactive notifications)</span></label>
              <input id="webhook-url-self" type="text" placeholder="https://hooks.slack.com/services/..." bind:value={webhookUrl} />
              <a href="https://api.slack.com/apps" target="_blank" class="link">Get it from api.slack.com/apps → Your App → Incoming Webhooks</a>
            </div>
            <a href="https://api.slack.com/apps" target="_blank" class="link">How to create a Slack app →</a>
          </div>
        {/if}
      </label>

      <label class="card" class:selected={mode === 'managed'}>
        <div class="card-header">
          <input type="radio" name="mode" value="managed" bind:group={mode} />
          <span class="card-title">Use managed bot</span>
          <span class="card-tag">connect to a shared routing service</span>
        </div>
        <p class="card-desc">Zero Slack app setup — connect via a shared bot</p>
        {#if mode === 'managed'}
          <div class="fields">
            <div class="field">
              <label for="router-url">Router URL</label>
              <input id="router-url" type="text" placeholder="https://router.izzie.dev" bind:value={routerUrl} />
            </div>
            <div class="field">
              <label for="router-token">Auth Token <span class="label-note">(provided by your admin)</span></label>
              <input id="router-token" type="password" placeholder="izzie_tok_________________________" bind:value={routerToken} />
            </div>
            <div class="field">
              <label for="webhook-url-managed">Incoming Webhook URL <span class="label-note">(optional — proactive notifications)</span></label>
              <input id="webhook-url-managed" type="text" placeholder="https://hooks.slack.com/services/..." bind:value={webhookUrl} />
              <a href="https://api.slack.com/apps" target="_blank" class="link">Get it from api.slack.com/apps → Your App → Incoming Webhooks</a>
            </div>
            <p class="note">Your token is issued by the router admin — it links your Slack identity to this instance.</p>
          </div>
        {/if}
      </label>

      <label class="card" class:selected={mode === 'skip'}>
        <div class="card-header">
          <input type="radio" name="mode" value="skip" bind:group={mode} />
          <span class="card-title">Skip for now</span>
        </div>
        <p class="card-desc">Set up Slack later by editing .env</p>
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
    gap: 8px;
    flex-wrap: wrap;
  }
  .card-title { font-weight: 600; color: #111; }
  .card-tag {
    font-size: 11px;
    color: #6b7280;
    font-style: italic;
  }
  .card-desc { color: #6b7280; font-size: 13px; margin: 0; }
  .fields { display: flex; flex-direction: column; gap: 10px; margin-top: 4px; }
  .field { display: flex; flex-direction: column; gap: 6px; }
  .field label { font-size: 12px; font-weight: 500; color: #374151; }
  .label-note { font-weight: 400; color: #9ca3af; }
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
