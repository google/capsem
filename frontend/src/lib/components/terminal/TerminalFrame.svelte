<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { WebglAddon } from '@xterm/addon-webgl';
  import '@xterm/xterm/css/xterm.css';
  import { TERMINAL_OPTIONS } from '../../terminal/terminal-config';
  import { getTheme, DEFAULT_THEME } from '../../terminal/themes';
  import { parseParentMessage } from '../../terminal/postmessage';
  import type { ParentToIframeMsg } from '../../terminal/postmessage';

  let containerEl: HTMLDivElement;
  let terminal: Terminal | null = null;
  let fitAddon: FitAddon | null = null;
  let resizeObserver: ResizeObserver | null = null;
  let resizeRafId = 0;
  let vmId: string | null = null;
  let ws: WebSocket | null = null;
  let wsConnected = false;
  let wsUrl: string | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let reconnectAttempt = 0;
  const MAX_RECONNECT_ATTEMPTS = 10;
  let destroyed = false;

  function applySettings(msg: { terminalTheme: string; mode: string; fontSize?: number; fontFamily?: string }): void {
    if (!terminal) return;
    // Theme
    terminal.options.theme = getTheme(msg.terminalTheme);
    const bg = terminal.options.theme?.background ?? '#000';
    document.body.style.backgroundColor = bg;
    // Font
    if (msg.fontSize && msg.fontSize >= 8 && msg.fontSize <= 32) {
      terminal.options.fontSize = msg.fontSize;
    }
    if (msg.fontFamily) {
      terminal.options.fontFamily = msg.fontFamily;
    }
    // Refit + repaint
    fitAddon?.fit();
    terminal.refresh(0, terminal.rows - 1);
  }

  function onMessage(event: MessageEvent): void {
    if (event.source !== window.parent) return;
    const msg = parseParentMessage(event.data);
    if (!msg) return;

    switch (msg.type) {
      case 'vm-id':
        vmId = msg.vmId;
        break;
      case 'theme-change':
        applySettings(msg);
        break;
      case 'focus':
        terminal?.focus();
        break;
      case 'clipboard-paste':
        terminal?.paste(msg.text);
        break;
      case 'ws-ticket':
        connectWebSocket(msg.ticket);
        break;
    }
  }

  function scheduleReconnect(): void {
    if (destroyed || !wsUrl || reconnectAttempt >= MAX_RECONNECT_ATTEMPTS) return;
    // Exponential backoff: 500ms, 1s, 2s, 4s, ... capped at 5s
    const delay = Math.min(500 * Math.pow(2, reconnectAttempt), 5000);
    reconnectAttempt++;
    if (terminal) {
      terminal.write(`\r\n\x1b[33m[Connecting... attempt ${reconnectAttempt}/${MAX_RECONNECT_ATTEMPTS}]\x1b[0m\r\n`);
    }
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      if (!destroyed && wsUrl) connectWebSocket(wsUrl);
    }, delay);
  }

  function connectWebSocket(url: string): void {
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    if (ws) {
      ws.close();
      ws = null;
    }
    wsUrl = url;

    try {
      const socket = new WebSocket(url);
      socket.binaryType = 'arraybuffer';
      let receivedData = false;

      socket.onopen = () => {
        wsConnected = true;
        reconnectAttempt = 0;
        if (terminal) {
          terminal.clear();
          if (fitAddon) {
            const dims = fitAddon.proposeDimensions();
            if (dims) {
              socket.send(JSON.stringify({ type: 'resize', cols: dims.cols, rows: dims.rows }));
            }
          }
          // Nudge the shell to redraw its prompt (it was printed before we connected)
          socket.send(new TextEncoder().encode('\n'));
        }
      };

      socket.onmessage = (event: MessageEvent) => {
        if (!terminal) return;
        receivedData = true;
        if (event.data instanceof ArrayBuffer) {
          terminal.write(new Uint8Array(event.data));
        } else {
          terminal.write(event.data);
        }
      };

      socket.onclose = () => {
        wsConnected = false;
        ws = null;
        if (!receivedData && reconnectAttempt < MAX_RECONNECT_ATTEMPTS) {
          // VM probably still booting -- retry
          scheduleReconnect();
        } else {
          if (terminal) {
            terminal.write('\r\n\x1b[1;31m[Connection closed]\x1b[0m\r\n');
          }
          sendToParent({ type: 'error', code: 'ws-closed', message: 'WebSocket connection closed' });
        }
      };

      socket.onerror = () => {
        // onerror is always followed by onclose, so reconnect logic runs there
      };

      ws = socket;
    } catch {
      sendToParent({ type: 'error', code: 'ws-failed', message: 'Failed to create WebSocket' });
    }
  }

  function sendToParent(msg: any): void {
    window.parent.postMessage(msg, '*');
  }

  onMount(async () => {
    if (!containerEl) return;
    window.addEventListener('message', onMessage);

    await document.fonts.ready;

    terminal = new Terminal({
      ...TERMINAL_OPTIONS,
      theme: getTheme(DEFAULT_THEME),
    });

    fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(containerEl);

    // GPU-accelerated rendering with canvas fallback
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose());
      terminal.loadAddon(webgl);
    } catch {
      // canvas fallback -- no action needed
    }

    fitAddon.fit();

    // ResizeObserver with rAF debounce
    resizeObserver = new ResizeObserver(() => {
      if (resizeRafId) cancelAnimationFrame(resizeRafId);
      resizeRafId = requestAnimationFrame(() => {
        resizeRafId = 0;
        fitAddon?.fit();
      });
    });
    resizeObserver.observe(containerEl);

    // Report resize to parent and WebSocket
    terminal.onResize(({ cols, rows }) => {
      sendToParent({ type: 'terminal-resize', cols, rows });
      if (wsConnected && ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: 'resize', cols, rows }));
      }
    });

    // Sanitize title changes before forwarding
    terminal.onTitleChange((title: string) => {
      const sanitized = title.replace(/[\x00-\x1f\x7f-\x9f]/g, '').slice(0, 128);
      sendToParent({ type: 'title-update', title: sanitized });
    });

    terminal.onData((data: string) => {
      if (wsConnected && ws && ws.readyState === WebSocket.OPEN) {
        ws.send(new TextEncoder().encode(data));
      }
    });

    terminal.focus();

    // Tell parent we're ready -- triggers vm-id and theme-change
    sendToParent({ type: 'ready' });
  });

  onDestroy(() => {
    destroyed = true;
    window.removeEventListener('message', onMessage);
    if (resizeRafId) cancelAnimationFrame(resizeRafId);
    if (reconnectTimer) { clearTimeout(reconnectTimer); reconnectTimer = null; }
    resizeObserver?.disconnect();
    if (ws) { ws.close(); ws = null; }
    terminal?.dispose();
  });
</script>

<div id="terminal-container" bind:this={containerEl}></div>

<style>
  #terminal-container {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    left: 0;
  }
</style>
