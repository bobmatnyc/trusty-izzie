<script lang="ts">
  type Message = { role: 'user' | 'assistant'; text: string; ts: number }

  let messages = $state<Message[]>([])
  let input = $state('')
  let sending = $state(false)
  let listEl = $state<HTMLElement | null>(null)

  function relativeTime(ts: number): string {
    const diff = Math.floor((Date.now() - ts) / 1000)
    if (diff < 10) return 'just now'
    if (diff < 60) return `${diff}s ago`
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
    return `${Math.floor(diff / 3600)}h ago`
  }

  async function send() {
    if (!input.trim() || sending) return
    const text = input.trim()
    input = ''
    sending = true
    messages = [...messages, { role: 'user', text, ts: Date.now() }]
    scrollToBottom()

    try {
      const res = await fetch('http://localhost:3456/api/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ message: text }),
      })
      const data = await res.json()
      messages = [
        ...messages,
        {
          role: 'assistant',
          text: data.response ?? data.message ?? JSON.stringify(data),
          ts: Date.now(),
        },
      ]
    } catch {
      messages = [
        ...messages,
        { role: 'assistant', text: 'Error: daemon not reachable on :3456', ts: Date.now() },
      ]
    } finally {
      sending = false
      scrollToBottom()
    }
  }

  function scrollToBottom() {
    // Use a microtask to scroll after DOM update
    Promise.resolve().then(() => {
      if (listEl) listEl.scrollTop = listEl.scrollHeight
    })
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      send()
    }
  }
</script>

<div class="chat">
  <div class="messages" bind:this={listEl}>
    {#if messages.length === 0}
      <div class="empty-state">
        <div class="empty-icon">💬</div>
        <p>Send a message to chat with Izzie</p>
      </div>
    {:else}
      {#each messages as msg}
        <div class="message-wrap" class:user={msg.role === 'user'}>
          <div class="bubble" class:user={msg.role === 'user'}>
            {msg.text}
            <span class="ts">{relativeTime(msg.ts)}</span>
          </div>
        </div>
      {/each}
      {#if sending}
        <div class="message-wrap">
          <div class="bubble typing">
            <span class="dot"></span>
            <span class="dot"></span>
            <span class="dot"></span>
          </div>
        </div>
      {/if}
    {/if}
  </div>

  <div class="input-bar">
    <textarea
      placeholder="Message Izzie… (Enter to send, Shift+Enter for newline)"
      bind:value={input}
      onkeydown={handleKeydown}
      rows={1}
      disabled={sending}
    ></textarea>
    <button onclick={send} disabled={sending || !input.trim()}>
      {sending ? '…' : '↑'}
    </button>
  </div>
</div>

<style>
  .chat {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .messages {
    flex: 1;
    overflow-y: auto;
    padding: 24px 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    scroll-behavior: smooth;
  }

  .empty-state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: #9ca3af;
    font-size: 14px;
    padding-top: 80px;
  }

  .empty-icon {
    font-size: 36px;
    opacity: 0.5;
  }

  .message-wrap {
    display: flex;
    justify-content: flex-start;
  }

  .message-wrap.user {
    justify-content: flex-end;
  }

  .bubble {
    max-width: 72%;
    padding: 10px 14px;
    border-radius: 16px;
    font-size: 14px;
    line-height: 1.5;
    background: white;
    border: 1px solid #e5e7eb;
    color: #111827;
    position: relative;
    word-break: break-word;
    white-space: pre-wrap;
  }

  .bubble.user {
    background: #2563eb;
    color: white;
    border-color: #2563eb;
    border-bottom-right-radius: 4px;
  }

  .bubble:not(.user) {
    border-bottom-left-radius: 4px;
  }

  .ts {
    display: none;
    font-size: 10px;
    opacity: 0.6;
    margin-left: 8px;
    white-space: nowrap;
  }

  .bubble:hover .ts {
    display: inline;
  }

  .bubble.typing {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 12px 16px;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #9ca3af;
    animation: bounce 1.2s infinite;
  }

  .dot:nth-child(2) { animation-delay: 0.2s; }
  .dot:nth-child(3) { animation-delay: 0.4s; }

  @keyframes bounce {
    0%, 60%, 100% { transform: translateY(0); }
    30% { transform: translateY(-4px); }
  }

  .input-bar {
    display: flex;
    align-items: flex-end;
    gap: 10px;
    padding: 16px 20px;
    border-top: 1px solid #e5e7eb;
    background: white;
  }

  textarea {
    flex: 1;
    resize: none;
    border: 1px solid #d1d5db;
    border-radius: 10px;
    padding: 10px 14px;
    font-size: 14px;
    font-family: inherit;
    line-height: 1.5;
    max-height: 120px;
    overflow-y: auto;
    background: #f9fafb;
    color: #111827;
    transition: border-color 0.15s;
  }

  textarea:focus {
    outline: none;
    border-color: #2563eb;
    background: white;
  }

  textarea:disabled {
    opacity: 0.5;
  }

  button {
    width: 36px;
    height: 36px;
    border-radius: 8px;
    border: none;
    background: #2563eb;
    color: white;
    font-size: 16px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    transition: background 0.15s;
  }

  button:hover:not(:disabled) {
    background: #1d4ed8;
  }

  button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
