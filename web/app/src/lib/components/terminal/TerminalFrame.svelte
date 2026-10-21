<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { StreamKind, MAX_STREAM_DATA_BYTES } from '@capsem/sdk';
  import { initTauriLog } from '../../tauri-log';
  import { gatewayBaseUrl } from '../../gateway-url';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { WebglAddon } from '@xterm/addon-webgl';
  import { WebLinksAddon } from '@xterm/addon-web-links';
  import '@xterm/xterm/css/xterm.css';
  import { TERMINAL_OPTIONS } from '../../terminal/terminal-config';
  import { getTheme, DEFAULT_THEME } from '../../terminal/themes';
  import { TerminalInputCoalescer } from '../../terminal/io-coalescer';
  import { SessionProjection } from '../../terminal/session-projection';
  import { parseParentMessage } from '../../terminal/postmessage';
  import { STREAM_SUBPROTOCOL, decodeServerFrame, encodeControl, encodeStdin, streamUrl } from '../../terminal/stream-protocol';

  initTauriLog();

  const GATEWAY_HTTP = gatewayBaseUrl();
  const GATEWAY_WS = GATEWAY_HTTP.replace(/^http/, 'ws');

  const MAX_RECONNECT = 10;

  let containerEl: HTMLDivElement;
  let terminal: Terminal | null = null;
  let fitAddon: FitAddon | null = null;
  let resizeObserver: ResizeObserver | null = null;
  let resizeRafId = 0;

  let ws: WebSocket | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let reconnectAttempt = $state(0);
  let destroyed = false;
  let connectionGeneration = 0;
  let everConnected = false;
  let projection: SessionProjection | null = null;
  let inputCoalescer: TerminalInputCoalescer | null = null;

  // Reactive overlay state -- drives the loading animation shown on top of
  // the terminal while we're waiting on the gateway / VM boot.
  let overlayVisible = $state(true);
  let overlayText = $state('Setting up session...');

  // Read initial state from URL. No postMessage handshake -- iframe owns its
  // setup data via its own URL.
  const params = typeof window !== 'undefined'
    ? new URLSearchParams(window.location.search)
    : new URLSearchParams();
  const vmId = params.get('vm') ?? '';
  const initialTheme = params.get('theme') ?? DEFAULT_THEME;
  const initialMode = (params.get('mode') === 'light' ? 'light' : 'dark') as 'light' | 'dark';
  const initialFontSize = Math.max(8, Math.min(32, Number(params.get('fontSize')) || 14));
  const initialFontFamily = params.get('fontFamily') ?? '';

  // Sync the body background to the terminal theme BEFORE first paint so the
  // overlay (which inherits body bg) matches the eventual terminal surface.
  // Otherwise the overlay flashes in a different color than the xterm pane.
  if (typeof document !== 'undefined') {
    const bg = getTheme(initialTheme)?.background ?? '#000';
    document.documentElement.style.backgroundColor = bg;
    document.body.style.backgroundColor = bg;
  }

  function postToParent(msg: unknown): void {
    try { window.parent.postMessage(msg, '*'); } catch { /* detached */ }
  }

  function openTerminalLink(event: MouseEvent, url: string): void {
    event.preventDefault();
    // The sandbox cannot open browser tabs itself. Its parent validates the
    // URL and uses the same native/browser opener as the rest of the app.
    postToParent({ type: 'open-url', url });
  }

  function requestDimensions(focus = false): void {
    if (ws?.readyState !== WebSocket.OPEN || !containerEl || containerEl.clientWidth <= 0 || containerEl.clientHeight <= 0 || document.hidden) return;
    const dims = fitAddon?.proposeDimensions();
    if (!dims || dims.cols <= 0 || dims.rows <= 0) return;
    // Layout requests a size. Only the session's acknowledgement sizes xterm.
    ws.send(encodeControl({ type: focus ? 'focus' : 'resize', cols: dims.cols, rows: dims.rows }));
  }

  function onWindowFocus(): void { requestDimensions(true); }

  function sendTerminalBytes(bytes: Uint8Array): void {
    if (ws?.readyState === WebSocket.OPEN) {
      for (let offset = 0; offset < bytes.length; offset += MAX_STREAM_DATA_BYTES) {
        ws.send(encodeStdin(bytes.subarray(offset, offset + MAX_STREAM_DATA_BYTES)));
      }
    }
  }

  async function fetchToken(): Promise<string | null> {
    try {
      const resp = await fetch(`${GATEWAY_HTTP}/token`);
      if (!resp.ok) return null;
      const data = await resp.json();
      return typeof data.token === 'string' ? data.token : null;
    } catch (e) {
      console.error('[terminal] token fetch failed', e);
      return null;
    }
  }

  function showOverlay(text: string): void {
    overlayText = text;
    overlayVisible = true;
  }

  function hideOverlay(): void {
    overlayVisible = false;
  }

  function scheduleReconnect(reason: string): void {
    if (destroyed) return;
    postToParent({ type: 'disconnected', reason });
    if (reconnectAttempt >= MAX_RECONNECT) {
      showOverlay('Connection failed');
      return;
    }
    const delay = Math.min(500 * Math.pow(2, reconnectAttempt), 5000);
    reconnectAttempt++;
    console.log('[terminal] reconnect attempt=%d delay=%dms reason=%s', reconnectAttempt, delay, reason);
    showOverlay(everConnected ? 'Reconnecting...' : 'Setting up session...');
    if (reconnectTimer) clearTimeout(reconnectTimer);
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      void connect();
    }, delay);
  }

  function retryConnection(): void {
    reconnectAttempt = 0;
    showOverlay(everConnected ? 'Reconnecting...' : 'Setting up session...');
    void connect();
  }

  async function connect(): Promise<void> {
    if (destroyed) return;
    const generation = ++connectionGeneration;
    inputCoalescer?.reset();
    if (!vmId) {
      terminal?.write('\r\n\x1b[1;31m[No VM ID provided]\x1b[0m\r\n');
      postToParent({ type: 'error', code: 'token-failed', message: 'missing vm id' });
      return;
    }
    if (ws) {
      try { ws.close(); } catch { /* already closed */ }
      ws = null;
    }

    const token = await fetchToken();
    if (destroyed || generation !== connectionGeneration) return;
    if (!token) {
      scheduleReconnect('token fetch failed');
      return;
    }
    const url = streamUrl(GATEWAY_WS, vmId, token);
    console.log('[terminal] connecting vmId=%s', vmId);

    let socket: WebSocket;
    try {
      socket = new WebSocket(url, [STREAM_SUBPROTOCOL]);
    } catch (e) {
      console.error('[terminal] WebSocket construction failed', e);
      scheduleReconnect('ws construction failed');
      return;
    }
    socket.binaryType = 'arraybuffer';

    socket.onopen = () => {
      if (socket !== ws || destroyed) return;
      console.log('[terminal] ws-open vmId=%s', vmId);
      reconnectAttempt = 0;
      postToParent({ type: 'connected' });
      socket.send(encodeControl({ type: 'start', kind: StreamKind.TERMINAL }));
      projection?.reconnect();
      requestDimensions(true);
      // No keystroke is injected to "redraw the prompt": a newline typed into
      // whatever the user left running (an editor, a half-typed command) is
      // input they never gave. The session sends its current display instead.
      // NOTE: do not flip `everConnected` here. The gateway sometimes
      // completes the WS handshake and closes moments later when the
      // per-VM UDS isn't ready yet. Using that as "first connect" makes
      // the next retry say "Reconnecting..." even though the VM never
      // actually gave us a prompt. We only count it as connected once
      // the first byte of real PTY output arrives (onmessage below).
    };

    socket.onmessage = (event: MessageEvent) => {
      if (!terminal || !(event.data instanceof ArrayBuffer)) return;
      const frame = decodeServerFrame(event.data);
      if (socket !== ws) return;
      if (frame.kind === 'status') {
        if (frame.status.type === 'terminal') {
          projection?.state(frame.status.state);
          postToParent({ type: 'title-update', title: frame.status.state.title });
        }
        if (frame.status.type === 'error') {
          console.warn('[terminal] stream ended vmId=%s reason=%s', vmId, frame.status.message);
        }
        return;
      }
      if (frame.kind === 'invalid') {
        console.error('[terminal] %s', frame.reason);
        socket.close(1002, 'invalid terminal state');
        return;
      }
      if (!everConnected) {
        everConnected = true;
        console.log('[terminal] first-data vmId=%s', vmId);
      }
      hideOverlay();
      projection?.output(frame.bytes);
    };

    socket.onclose = (ev) => {
      console.log('[terminal] closed vmId=%s code=%d', vmId, ev.code);
      if (socket !== ws) return;
      ws = null;
      scheduleReconnect(`ws closed code=${ev.code}`);
    };

    socket.onerror = () => {
      console.warn('[terminal] ws error vmId=%s', vmId);
      // onclose will follow; reconnect handled there.
    };

    ws = socket;
  }

  function applySettings(theme: string, mode: 'light' | 'dark', fontSize: number, fontFamily: string): void {
    if (!terminal) return;
    // theme is the resolved terminal theme name (e.g., 'github-dark'); mode is kept
    // for background-syncing only in case the theme name isn't in the registry.
    void mode;
    terminal.options.theme = getTheme(theme);
    const bg = terminal.options.theme?.background ?? '#000';
    document.body.style.backgroundColor = bg;
    if (fontSize >= 8 && fontSize <= 32) terminal.options.fontSize = fontSize;
    if (fontFamily) terminal.options.fontFamily = fontFamily;
    requestDimensions();
    terminal.refresh(0, terminal.rows - 1);
  }

  function onMessage(event: MessageEvent): void {
    if (event.source !== window.parent) return;
    const msg = parseParentMessage(event.data);
    if (!msg) return;
    switch (msg.type) {
      case 'theme-change':
        applySettings(msg.terminalTheme, msg.mode, msg.fontSize, msg.fontFamily);
        break;
      case 'focus':
        terminal?.focus();
        requestDimensions(true);
        break;
      case 'clipboard-paste':
        terminal?.paste(msg.text);
        break;
    }
  }

  onMount(async () => {
    window.addEventListener('message', onMessage);
    window.addEventListener('focus', onWindowFocus);

    await document.fonts.ready;

    terminal = new Terminal({
      ...TERMINAL_OPTIONS,
      theme: getTheme(initialTheme),
      fontSize: initialFontSize,
      linkHandler: { activate: openTerminalLink },
      ...(initialFontFamily ? { fontFamily: initialFontFamily } : {}),
    });
    document.body.style.backgroundColor = terminal.options.theme?.background ?? '#000';

    fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.loadAddon(new WebLinksAddon(openTerminalLink));
    terminal.open(containerEl);
    projection = new SessionProjection(terminal, () => ws?.close(1000, 'resync current display'));
    inputCoalescer = new TerminalInputCoalescer(sendTerminalBytes);

    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose());
      terminal.loadAddon(webgl);
    } catch {
      // canvas fallback, no action
    }

    fitAddon.fit();

    resizeObserver = new ResizeObserver(() => {
      if (resizeRafId) cancelAnimationFrame(resizeRafId);
      resizeRafId = requestAnimationFrame(() => {
        resizeRafId = 0;
        requestDimensions();
      });
    });
    resizeObserver.observe(containerEl);

    terminal.onData((data: string) => {
      inputCoalescer?.push(new TextEncoder().encode(data));
    });

    containerEl.addEventListener('focusin', () => requestDimensions(true));
    terminal.focus();

    // Mark initial mode unused (already wired via theme name).
    void initialMode;

    await connect();
  });

  onDestroy(() => {
    destroyed = true;
    connectionGeneration++;
    window.removeEventListener('message', onMessage);
    window.removeEventListener('focus', onWindowFocus);
    if (resizeRafId) cancelAnimationFrame(resizeRafId);
    if (reconnectTimer) { clearTimeout(reconnectTimer); reconnectTimer = null; }
    resizeObserver?.disconnect();
    if (ws) { try { ws.close(); } catch { /* already closed */ } ws = null; }
    projection?.reconnect();
    inputCoalescer?.reset();
    terminal?.dispose();
  });
</script>

<div id="terminal-container" bind:this={containerEl}></div>

{#if overlayVisible}
  <div class="loading-overlay" role="status" aria-live="polite">
    {#if reconnectAttempt < MAX_RECONNECT}
    <div class="spinner" aria-hidden="true">
      <div class="dot"></div>
      <div class="dot"></div>
      <div class="dot"></div>
    </div>
    {/if}
    <div class="loading-text">{overlayText}</div>
    {#if reconnectAttempt >= MAX_RECONNECT}
      <button type="button" class="retry-button" onclick={retryConnection}>Retry connection</button>
    {/if}
  </div>
{/if}

<style>
  #terminal-container {
    position: absolute;
    top: 10px;
    right: 10px;
    bottom: 10px;
    left: 10px;
  }

  .loading-overlay {
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 1.25rem;
    /* Solid background -- inherits body color which TerminalFrame.onMount
       syncs to the terminal theme background. No transparency, no fade
       animation: the overlay and the terminal are visually the same
       surface, so appearing/disappearing is instantaneous and silent. */
    background: inherit;
    color: #cbd5e1;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif;
    z-index: 10;
  }

  .loading-text {
    font-size: 0.875rem;
    letter-spacing: 0.01em;
    opacity: 0.85;
  }

  .spinner {
    display: flex;
    gap: 0.5rem;
  }

  .retry-button {
    border: 1px solid #60a5fa;
    border-radius: 0.5rem;
    background: transparent;
    color: inherit;
    padding: 0.5rem 1rem;
    cursor: pointer;
  }

  .retry-button:focus-visible {
    outline: 2px solid #60a5fa;
    outline-offset: 2px;
  }

  .spinner .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: #60a5fa;
    animation: pulse 1.1s ease-in-out infinite;
  }

  .spinner .dot:nth-child(2) { animation-delay: 0.15s; }
  .spinner .dot:nth-child(3) { animation-delay: 0.3s; }

  @keyframes pulse {
    0%, 100% { transform: scale(0.6); opacity: 0.35; }
    50%      { transform: scale(1);   opacity: 1;    }
  }

</style>
