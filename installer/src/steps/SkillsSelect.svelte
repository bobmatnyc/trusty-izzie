<script lang="ts">
  import type { SlackConfig } from './SlackSetup.svelte'

  export type SkillsConfig = {
    metroNorth: boolean
    imessage: boolean
    tavily: boolean
    tavilyApiKey?: string
    slackSearch: boolean
  }

  let { onNext, onBack, onUpdate, slackConfig } = $props<{
    onNext: () => void
    onBack: () => void
    onUpdate: (cfg: SkillsConfig) => void
    slackConfig: SlackConfig
  }>()

  let metroNorth = $state(false)
  let imessage = $state(true)
  let tavily = $state(false)
  let tavilyApiKey = $state('')
  let slackSearch = $state(slackConfig.mode !== 'skip')

  // Keep slackSearch in sync if parent changes
  $effect(() => {
    if (slackConfig.mode !== 'skip') slackSearch = true
  })

  const bundled = [
    { label: 'Web search', desc: 'Search the web via Brave' },
    { label: 'Weather', desc: 'Current conditions and forecasts' },
    { label: 'Calendar & Tasks', desc: 'Read and create Google Calendar events and Tasks' },
    { label: 'Email', desc: 'Search sent mail, compose and send (with approval)' },
    { label: 'Memory search', desc: 'Search your contacts, companies, and projects' },
    { label: 'Morning briefing', desc: 'Daily digest of calendar, tasks, and news' },
  ]

  function handleNext() {
    const cfg: SkillsConfig = {
      metroNorth,
      imessage,
      tavily,
      slackSearch,
    }
    if (tavily && tavilyApiKey.trim()) cfg.tavilyApiKey = tavilyApiKey.trim()
    onUpdate(cfg)
    onNext()
  }
</script>

<div class="step">
  <div class="content">
    <h2>Skills</h2>
    <p class="subtitle">Choose which capabilities to enable</p>

    <div class="section">
      <div class="section-title">Bundled</div>
      {#each bundled as skill}
        <div class="skill-row bundled">
          <span class="check">✓</span>
          <div>
            <span class="skill-name">{skill.label}</span>
            <span class="skill-desc"> — {skill.desc}</span>
          </div>
        </div>
      {/each}
    </div>

    <div class="section">
      <div class="section-title">Optional</div>

      <label class="skill-row optional">
        <input type="checkbox" bind:checked={metroNorth} />
        <div class="skill-info">
          <span class="skill-name">Metro North trains</span>
          <span class="skill-desc"> — MTA train schedules and live track assignments</span>
          <span class="tag regional">Regional</span>
        </div>
      </label>

      <label class="skill-row optional">
        <input type="checkbox" bind:checked={imessage} />
        <div class="skill-info">
          <span class="skill-name">iMessage search</span>
          <span class="skill-desc"> — Search your Messages app (macOS only)</span>
          <span class="tag macos">macOS</span>
        </div>
      </label>

      <label class="skill-row optional">
        <input type="checkbox" bind:checked={tavily} />
        <div class="skill-info">
          <span class="skill-name">Tavily search</span>
          <span class="skill-desc"> — Enhanced web research with full-page answers</span>
          <span class="tag apikey">API key required</span>
        </div>
      </label>
      {#if tavily}
        <div class="inline-field">
          <input type="password" placeholder="Tavily API key" bind:value={tavilyApiKey} />
        </div>
      {/if}

      <label class="skill-row optional" class:auto-enabled={slackConfig.mode !== 'skip'}>
        <input type="checkbox" bind:checked={slackSearch} />
        <div class="skill-info">
          <span class="skill-name">Slack search</span>
          <span class="skill-desc"> — Search Slack messages</span>
          <span class="tag slack">Requires Slack</span>
          {#if slackConfig.mode !== 'skip'}
            <span class="auto-note">auto-enabled</span>
          {/if}
        </div>
      </label>
    </div>

    <div class="actions">
      <button class="secondary" onclick={onBack}>← Back</button>
      <button onclick={handleNext}>Continue →</button>
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
    overflow-y: auto;
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
  .section { display: flex; flex-direction: column; gap: 4px; }
  .section-title {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: #9ca3af;
    margin-bottom: 8px;
  }
  .skill-row {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 12px;
    border-radius: 8px;
    background: white;
    border: 1px solid #f3f4f6;
  }
  .skill-row.bundled { cursor: default; }
  .skill-row.optional { cursor: pointer; }
  .skill-row.optional:hover { background: #f9fafb; }
  .skill-row.auto-enabled { background: #eff6ff; border-color: #bfdbfe; }
  .check { color: #10b981; font-weight: 700; font-size: 14px; flex-shrink: 0; padding-top: 1px; }
  .skill-info { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; }
  .skill-name { font-weight: 500; color: #111; font-size: 14px; }
  .skill-desc { color: #6b7280; font-size: 13px; }
  .tag {
    font-size: 10px;
    font-weight: 500;
    padding: 2px 7px;
    border-radius: 10px;
    flex-shrink: 0;
  }
  .tag.regional { background: #fef9c3; color: #854d0e; }
  .tag.macos { background: #f3f4f6; color: #374151; }
  .tag.apikey { background: #fef3c7; color: #92400e; }
  .tag.slack { background: #ede9fe; color: #5b21b6; }
  .auto-note { font-size: 11px; color: #2563eb; font-style: italic; }
  input[type="checkbox"] { accent-color: #2563eb; width: 16px; height: 16px; flex-shrink: 0; margin-top: 2px; }
  .inline-field { padding: 0 12px 8px 38px; }
  .inline-field input {
    border: 1px solid #d1d5db;
    border-radius: 8px;
    padding: 8px 12px;
    font-size: 13px;
    font-family: inherit;
    width: 100%;
    box-sizing: border-box;
  }
  .inline-field input:focus {
    outline: none;
    border-color: #2563eb;
    box-shadow: 0 0 0 3px rgba(37,99,235,0.1);
  }
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
  button:hover { background: #1d4ed8; }
  button.secondary { background: #f3f4f6; color: #374151; }
  button.secondary:hover { background: #e5e7eb; }
</style>
