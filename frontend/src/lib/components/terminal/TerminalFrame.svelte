<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { initTauriLog } from '../../tauri-log';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { WebglAddon } from '@xterm/addon-webgl';
  import '@xterm/xterm/css/xterm.css';
  import { TERMINAL_OPTIONS } from '../../terminal/terminal-config';
  import { getTheme, DEFAULT_THEME } from '../../terminal/themes';
  import { parseParentMessage } from '../../terminal/postmessage';

  initTauriLog();

  // Gateway base URLs. Default to the standard gateway port; override with
  // ?gw=... URL param for dev/test if ever needed.
  const GATEWAY_HTTP = 'http://127.0.0.1:19222';
  const GATEWAY_WS = 'ws://127.0.0.1:19222';

  const MAX_RECONNECT = 10;

  let containerEl: HTMLDivElement;
  let terminal: Terminal | null = null;
  let fitAddon: FitAddon | null = null;
  let resizeObserver: ResizeObserver | null = null;
  let resizeRafId = 0;

  let ws: WebSocket | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let reconnectAttempt = 0;
  let destroyed = false;

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

  function postToParent(msg: unknown): void {
    try { window.parent.postMessage(msg, '*'); } catch { /* detached */ }
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

  function scheduleReconnect(reason: string): void {
    if (destroyed) return;
    postToParent({ type: 'disconnected', reason });
    if (reconnectAttempt >= MAX_RECONNECT) {
      terminal?.write(`\r\n\x1b[1;31m[Connection lost: ${reason}. Reload to retry.]\x1b[0m\r\n`);
      return;
    }
    const delay = Math.min(500 * Math.pow(2, reconnectAttempt), 5000);
    reconnectAttempt++;
    console.log('[terminal] reconnect attempt=%d delay=%dms', reconnectAttempt, delay);
    terminal?.write(`\r\n\x1b[33m[Reconnecting in ${Math.round(delay / 100) / 10}s...]\x1b[0m\r\n`);
    if (reconnectTimer) clearTimeout(reconnectTimer);
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      void connect();
    }, delay);
  }

  async function connect(): Promise<void> {
    if (destroyed) return;
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
    if (!token) {
      scheduleReconnect('token fetch failed');
      return;
    }
    const url = `${GATEWAY_WS}/terminal/${encodeURIComponent(vmId)}?token=${encodeURIComponent(token)}`;
    console.log('[terminal] connecting vmId=%s', vmId);

    let socket: WebSocket;
    try {
      socket = new WebSocket(url);
    } catch (e) {
      console.error('[terminal] WebSocket construction failed', e);
      scheduleReconnect('ws construction failed');
      return;
    }
    socket.binaryType = 'arraybuffer';

    socket.onopen = () => {
      console.log('[terminal] connected vmId=%s', vmId);
      reconnectAttempt = 0;
      postToParent({ type: 'connected' });
      if (terminal && fitAddon) {
        const dims = fitAddon.proposeDimensions();
        if (dims) {
          socket.send(JSON.stringify({ type: 'resize', cols: dims.cols, rows: dims.rows }));
        }
        // Nudge shell to redraw its prompt.
        socket.send(new TextEncoder().encode('\n'));
      }
    };

    socket.onmessage = (event: MessageEvent) => {
      if (!terminal) return;
      if (event.data instanceof ArrayBuffer) {
        terminal.write(new Uint8Array(event.data));
      } else {
        terminal.write(event.data);
      }
    };

    socket.onclose = (ev) => {
      console.log('[terminal] closed vmId=%s code=%d', vmId, ev.code);
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
    fitAddon?.fit();
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
        break;
      case 'clipboard-paste':
        terminal?.paste(msg.text);
        break;
    }
  }

  onMount(async () => {
    window.addEventListener('message', onMessage);

    await document.fonts.ready;

    terminal = new Terminal({
      ...TERMINAL_OPTIONS,
      theme: getTheme(initialTheme),
      fontSize: initialFontSize,
      ...(initialFontFamily ? { fontFamily: initialFontFamily } : {}),
    });
    document.body.style.backgroundColor = terminal.options.theme?.background ?? '#000';

    fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(containerEl);

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
        fitAddon?.fit();
      });
    });
    resizeObserver.observe(containerEl);

    terminal.onResize(({ cols, rows }) => {
      if (ws?.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: 'resize', cols, rows }));
      }
    });

    terminal.onTitleChange((title: string) => {
      const sanitized = title.replace(/[\x00-\x1f\x7f-\x9f]/g, '').slice(0, 128);
      postToParent({ type: 'title-update', title: sanitized });
    });

    terminal.onData((data: string) => {
      if (ws?.readyState === WebSocket.OPEN) {
        ws.send(new TextEncoder().encode(data));
      }
    });

    terminal.focus();

    // Mark initial mode unused (already wired via theme name).
    void initialMode;

    await connect();
  });

  onDestroy(() => {
    destroyed = true;
    window.removeEventListener('message', onMessage);
    if (resizeRafId) cancelAnimationFrame(resizeRafId);
    if (reconnectTimer) { clearTimeout(reconnectTimer); reconnectTimer = null; }
    resizeObserver?.disconnect();
    if (ws) { try { ws.close(); } catch { /* already closed */ } ws = null; }
    terminal?.dispose();
  });
</script>

<div id="terminal-container" bind:this={containerEl}></div>

<style>
  #terminal-container {
    position: absolute;
    top: 10px;
    right: 10px;
    bottom: 10px;
    left: 10px;
  }
</style>
