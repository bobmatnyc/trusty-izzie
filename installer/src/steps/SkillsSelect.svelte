<script lang="ts">
  import type { SlackConfig } from './SlackSetup.svelte'

  export type SkillKey = {
    env: string
    label: string
    placeholder: string
    required: boolean
    url?: string
    urlLabel?: string
    scopes?: string
  }

  export type SkillDef = {
    id: string
    name: string
    description: string
    scope_summary?: string
    tags?: string[]
    keys: SkillKey[]
  }

  export type SkillsManifest = {
    bundled: SkillDef[]
    optional: SkillDef[]
  }

  export type SkillsConfig = {
    enabled: string[]
    keys: Record<string, string>  // env_var → value
  }

  let { onNext, onBack, onUpdate, slack } = $props<{
    onNext: () => void
    onBack: () => void
    onUpdate: (cfg: SkillsConfig) => void
    slack: SlackConfig
  }>()

  let manifest = $state<SkillsManifest | null>(null)
  let enabled = $state<Set<string>>(new Set())
  let keyValues = $state<Record<string, string>>({})
  let loadError = $state<string | null>(null)

  // Load manifest on mount
  $effect(() => {
    fetch('/skills-manifest.json')
      .then(r => r.json())
      .then((m: SkillsManifest) => {
        manifest = m

        // Auto-enable all bundled skills
        const defaults = new Set(m.bundled.map((s: SkillDef) => s.id))

        // Auto-enable slack_search if Slack is configured
        if (slack.mode !== 'skip') {
          defaults.add('slack_search')
        }

        enabled = defaults
      })
      .catch((e: Error) => {
        loadError = e.message
      })
  })

  function toggle(id: string) {
    const next = new Set(enabled)
    if (next.has(id)) {
      next.delete(id)
      // Clear keys for this skill
      if (manifest) {
        const skill = manifest.optional.find((s: SkillDef) => s.id === id)
        if (skill) {
          const nextKeys = { ...keyValues }
          skill.keys.forEach((k: SkillKey) => delete nextKeys[k.env])
          keyValues = nextKeys
        }
      }
    } else {
      next.add(id)
    }
    enabled = next
  }

  function setKey(env: string, value: string) {
    keyValues = { ...keyValues, [env]: value }
  }

  function missingKeys(): string[] {
    if (!manifest) return []
    const missing: string[] = []
    const allSkills = [...manifest.bundled, ...manifest.optional]
    for (const skill of allSkills) {
      if (enabled.has(skill.id)) {
        for (const k of skill.keys) {
          if (k.required && !keyValues[k.env]?.trim()) missing.push(k.env)
        }
      }
    }
    return missing
  }

  function canContinue() {
    return manifest !== null && missingKeys().length === 0
  }

  function handleNext() {
    const cfg: SkillsConfig = {
      enabled: Array.from(enabled),
      keys: Object.fromEntries(
        Object.entries(keyValues).filter(([, v]) => v.trim().length > 0)
      ),
    }
    onUpdate(cfg)
    onNext()
  }
</script>

<div class="step">
  <div class="content">
    <h2>Skills</h2>
    <p class="subtitle">Choose which capabilities to enable</p>

    {#if loadError}
      <div class="error">Failed to load skills: {loadError}</div>
    {:else if !manifest}
      <div class="loading">Loading skills...</div>
    {:else}

      <!-- Bundled skills -->
      <section>
        <div class="section-header">
          <span class="section-title">Bundled</span>
          <span class="section-note">always enabled</span>
        </div>
        <div class="skill-list">
          {#each manifest.bundled as skill}
            <div class="skill-row bundled">
              <div class="skill-main">
                <span class="check-icon">✓</span>
                <div class="skill-info">
                  <span class="skill-name">{skill.name}</span>
                  <span class="skill-desc">{skill.description}</span>
                </div>
              </div>
              {#if skill.keys.length > 0}
                <div class="key-fields">
                  {#each skill.keys as key}
                    <div class="key-field">
                      <label for="key-{key.env}">
                        {key.label}
                        {#if key.required}<span class="required">*</span>{/if}
                      </label>
                      <input
                        id="key-{key.env}"
                        type="password"
                        placeholder={key.placeholder}
                        value={keyValues[key.env] ?? ''}
                        oninput={(e) => setKey(key.env, (e.target as HTMLInputElement).value)}
                      />
                      {#if key.url}
                        <a href={key.url} target="_blank" class="key-link">{key.urlLabel ?? key.url}</a>
                      {/if}
                      {#if key.scopes}
                        <span class="key-scopes">Required scopes: {key.scopes}</span>
                      {/if}
                    </div>
                  {/each}
                </div>
              {/if}
            </div>
          {/each}
        </div>
      </section>

      <!-- Optional skills -->
      <section>
        <div class="section-header">
          <span class="section-title">Optional</span>
          <span class="section-note">toggle on/off</span>
        </div>
        <div class="skill-list">
          {#each manifest.optional as skill}
            {@const isEnabled = enabled.has(skill.id)}
            <div class="skill-row optional" class:active={isEnabled}>
              <div class="skill-main">
                <input
                  type="checkbox"
                  id="skill-{skill.id}"
                  checked={isEnabled}
                  onchange={() => toggle(skill.id)}
                />
                <div class="skill-info">
                  <label for="skill-{skill.id}" class="skill-name">{skill.name}</label>
                  <span class="skill-desc">{skill.description}</span>
                  {#if skill.scope_summary}
                    <span class="skill-scope">{skill.scope_summary}</span>
                  {/if}
                  {#if skill.tags?.length}
                    <div class="tags">
                      {#each skill.tags as tag}
                        <span class="tag">{tag}</span>
                      {/each}
                    </div>
                  {/if}
                </div>
              </div>
              {#if isEnabled && skill.keys.length > 0}
                <div class="key-fields">
                  {#each skill.keys as key}
                    <div class="key-field">
                      <label for="key-opt-{key.env}">
                        {key.label}
                        {#if key.required}<span class="required">*</span>{/if}
                      </label>
                      <input
                        id="key-opt-{key.env}"
                        type="password"
                        placeholder={key.placeholder}
                        value={keyValues[key.env] ?? ''}
                        oninput={(e) => setKey(key.env, (e.target as HTMLInputElement).value)}
                      />
                      {#if key.url}
                        <a href={key.url} target="_blank" class="key-link">{key.urlLabel ?? key.url}</a>
                      {/if}
                      {#if key.scopes}
                        <span class="key-scopes">Required scopes: {key.scopes}</span>
                      {/if}
                    </div>
                  {/each}
                </div>
              {/if}
            </div>
          {/each}
        </div>
      </section>

    {/if}

    <div class="actions">
      <button class="secondary" onclick={onBack}>← Back</button>
      <button onclick={handleNext} disabled={!canContinue()}>Continue →</button>
    </div>
  </div>
</div>

<style>
  .step {
    flex: 1;
    overflow-y: auto;
    padding: 32px 40px;
  }
  .content {
    width: 100%;
    max-width: 560px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  h2 { font-size: 22px; font-weight: 600; color: #111; margin: 0; }
  .subtitle { color: #6b7280; margin: 0; }

  .loading { color: #9ca3af; font-size: 14px; text-align: center; padding: 24px; }
  .error { color: #dc2626; font-size: 13px; background: #fef2f2; padding: 12px; border-radius: 8px; }

  section { display: flex; flex-direction: column; gap: 8px; }
  .section-header {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding-bottom: 4px;
    border-bottom: 1px solid #f3f4f6;
  }
  .section-title { font-size: 11px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.08em; color: #374151; }
  .section-note { font-size: 11px; color: #9ca3af; }

  .skill-list { display: flex; flex-direction: column; gap: 4px; }

  .skill-row {
    border: 1px solid #e5e7eb;
    border-radius: 10px;
    padding: 12px 14px;
    background: white;
    display: flex;
    flex-direction: column;
    gap: 12px;
    transition: border-color 0.15s, background 0.15s;
  }
  .skill-row.bundled { background: #f9fafb; }
  .skill-row.optional.active {
    background: #eff6ff;
    border-color: #bfdbfe;
  }

  .skill-main {
    display: flex;
    align-items: flex-start;
    gap: 10px;
  }
  .check-icon { color: #10b981; font-size: 14px; margin-top: 1px; flex-shrink: 0; }
  input[type="checkbox"] { accent-color: #2563eb; margin-top: 2px; flex-shrink: 0; cursor: pointer; }

  .skill-info { display: flex; flex-direction: column; gap: 2px; flex: 1; min-width: 0; }
  .skill-name { font-size: 13px; font-weight: 600; color: #111; cursor: pointer; }
  .skill-desc { font-size: 12px; color: #6b7280; line-height: 1.4; }
  .skill-scope { font-size: 11px; color: #9ca3af; line-height: 1.4; font-style: italic; }

  .tags { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
  .tag {
    font-size: 10px;
    padding: 2px 7px;
    border-radius: 999px;
    background: #f3f4f6;
    color: #6b7280;
    border: 1px solid #e5e7eb;
  }

  .key-fields {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-top: 4px;
    border-top: 1px solid #e5e7eb;
  }
  .key-field { display: flex; flex-direction: column; gap: 5px; }
  .key-field label { font-size: 12px; font-weight: 500; color: #374151; }
  .required { color: #dc2626; margin-left: 2px; }
  .key-link { font-size: 11px; color: #2563eb; text-decoration: none; }
  .key-link:hover { text-decoration: underline; }
  .key-scopes { font-size: 11px; color: #9ca3af; font-style: italic; }

  input[type="password"] {
    border: 1px solid #d1d5db;
    border-radius: 7px;
    padding: 8px 12px;
    font-size: 12px;
    font-family: monospace;
    width: 100%;
    box-sizing: border-box;
  }
  input[type="password"]:focus {
    outline: none;
    border-color: #2563eb;
    box-shadow: 0 0 0 3px rgba(37,99,235,0.1);
  }

  .actions { display: flex; gap: 12px; padding-top: 4px; }
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
