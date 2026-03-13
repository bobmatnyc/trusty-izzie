<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'

  let { onNext, onBack, onUpdate } = $props<{
    onNext: () => void
    onBack: () => void
    onUpdate: (email: string) => void
  }>()

  type OAuthState = 'idle' | 'waiting' | 'done' | 'error'
  let state = $state<OAuthState>('idle')
  let authorizedEmail = $state<string | null>(null)
  let errorMsg = $state('')
  let pollTimer: ReturnType<typeof setInterval> | null = null

  async function startOAuth() {
    state = 'waiting'
    errorMsg = ''
    try {
      await invoke('start_google_oauth')
      pollTimer = setInterval(async () => {
        try {
          const result = await invoke<string | null>('poll_oauth_result')
          if (result) {
            clearInterval(pollTimer!)
            pollTimer = null
            authorizedEmail = result
            state = 'done'
            onUpdate(result)
          }
        } catch (e) {
          clearInterval(pollTimer!)
          pollTimer = null
          errorMsg = String(e)
          state = 'error'
        }
      }, 1000)
    } catch (e) {
      errorMsg = String(e)
      state = 'error'
    }
  }

  function retry() {
    state = 'idle'
    errorMsg = ''
    authorizedEmail = null
  }
</script>

<div class="step">
  <div class="content">
    <h2>Google Account</h2>
    <p class="subtitle">Izzie reads your Gmail and Calendar to build your knowledge base</p>

    <div class="permissions">
      <div class="perm-title">Izzie will request:</div>
      <ul>
        <li>Read-only access to Gmail (sent mail)</li>
        <li>Read your Google Calendar events</li>
        <li>Send email on your behalf (requires approval)</li>
      </ul>
    </div>

    {#if state === 'idle'}
      <button class="google-btn" onclick={startOAuth}>
        <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
          <path d="M17.64 9.2c0-.637-.057-1.251-.164-1.84H9v3.481h4.844c-.209 1.125-.843 2.078-1.796 2.716v2.258h2.908c1.702-1.567 2.684-3.874 2.684-6.615z" fill="#4285F4"/>
          <path d="M9 18c2.43 0 4.467-.806 5.956-2.18l-2.908-2.259c-.806.54-1.837.86-3.048.86-2.344 0-4.328-1.584-5.036-3.711H.957v2.332A8.997 8.997 0 0 0 9 18z" fill="#34A853"/>
          <path d="M3.964 10.71A5.41 5.41 0 0 1 3.682 9c0-.593.102-1.17.282-1.71V4.958H.957A8.996 8.996 0 0 0 0 9c0 1.452.348 2.827.957 4.042l3.007-2.332z" fill="#FBBC05"/>
          <path d="M9 3.58c1.321 0 2.508.454 3.44 1.345l2.582-2.58C13.463.891 11.426 0 9 0A8.997 8.997 0 0 0 .957 4.958L3.964 6.29C4.672 4.163 6.656 3.58 9 3.58z" fill="#EA4335"/>
        </svg>
        Sign in with Google →
      </button>

    {:else if state === 'waiting'}
      <div class="waiting">
        <div class="spinner"></div>
        <span>Waiting for browser authorization...</span>
      </div>

    {:else if state === 'done'}
      <div class="success">
        <span class="check">✓</span>
        <span>Authorized as <strong>{authorizedEmail}</strong></span>
      </div>

    {:else if state === 'error'}
      <div class="error">
        <p>Authorization failed: {errorMsg}</p>
        <button class="secondary" onclick={retry}>Try Again</button>
      </div>
    {/if}

    <div class="actions">
      <button class="secondary" onclick={onBack}>← Back</button>
      <button onclick={onNext} disabled={state !== 'done'}>Continue →</button>
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
    max-width: 480px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  h2 { font-size: 22px; font-weight: 600; color: #111; margin: 0; }
  .subtitle { color: #6b7280; margin: 0; }
  .permissions {
    background: #f9fafb;
    border: 1px solid #e5e7eb;
    border-radius: 12px;
    padding: 16px;
  }
  .perm-title { font-size: 13px; font-weight: 500; color: #374151; margin-bottom: 8px; }
  ul { margin: 0; padding-left: 20px; }
  li { font-size: 13px; color: #6b7280; line-height: 1.8; }
  .google-btn {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 24px;
    background: white;
    color: #374151;
    border: 1px solid #d1d5db;
    border-radius: 8px;
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.15s, box-shadow 0.15s;
    align-self: flex-start;
  }
  .google-btn:hover { background: #f9fafb; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }
  .waiting {
    display: flex;
    align-items: center;
    gap: 12px;
    color: #6b7280;
    font-size: 14px;
  }
  .spinner {
    width: 18px; height: 18px;
    border: 2px solid #e5e7eb;
    border-top-color: #2563eb;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    flex-shrink: 0;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  .success {
    display: flex;
    align-items: center;
    gap: 10px;
    color: #374151;
    font-size: 14px;
    background: #f0fdf4;
    border: 1px solid #bbf7d0;
    border-radius: 8px;
    padding: 12px 16px;
  }
  .check { color: #10b981; font-weight: 700; font-size: 16px; }
  .error {
    background: #fef2f2;
    border: 1px solid #fecaca;
    border-radius: 8px;
    padding: 12px 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .error p { color: #dc2626; font-size: 13px; margin: 0; }
  .actions { display: flex; gap: 12px; margin-top: 4px; }
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
  button.secondary { background: #f3f4f6; color: #374151; border: none; }
  button.secondary:hover { background: #e5e7eb; }
</style>
