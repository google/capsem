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
  let renderer: 'webgl' | 'canvas' = 'canvas';
  let resizeObserver: ResizeObserver | null = null;
  let resizeRafId = 0;
  let vmId: string | null = null;
  let ws: WebSocket | null = null;
  let wsConnected = false;
  let mockEchoEnabled = true;

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

  function connectWebSocket(url: string): void {
    if (ws) {
      ws.close();
      ws = null;
    }

    try {
      const socket = new WebSocket(url);
      socket.binaryType = 'arraybuffer';

      socket.onopen = () => {
        wsConnected = true;
        mockEchoEnabled = false;
        if (terminal) {
          terminal.clear();
          // Send current terminal size as first message
          if (fitAddon) {
            const dims = fitAddon.proposeDimensions();
            if (dims) {
              socket.send(JSON.stringify({ type: 'resize', cols: dims.cols, rows: dims.rows }));
            }
          }
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

      socket.onclose = () => {
        wsConnected = false;
        ws = null;
        if (terminal) {
          terminal.write('\r\n\x1b[1;31m[Connection closed]\x1b[0m\r\n');
        }
        sendToParent({ type: 'error', code: 'ws-closed', message: 'WebSocket connection closed' });
      };

      socket.onerror = () => {
        wsConnected = false;
        ws = null;
        sendToParent({ type: 'error', code: 'ws-failed', message: 'WebSocket connection failed' });
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
      webgl.onContextLoss(() => {
        webgl.dispose();
        renderer = 'canvas';
      });
      terminal.loadAddon(webgl);
      renderer = 'webgl';
    } catch {
      renderer = 'canvas';
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

    // Mock banner for Sprint 01
    const enc = new TextEncoder();
    terminal.write(enc.encode(
      '\x1b[1;34mCAPSEM sandbox ready\x1b[0m\r\n' +
      '\x1b[35mLinux 6.6.127 | aarch64\x1b[0m\r\n' +
      '\r\n' +
      'Renderer: ' + renderer + '\r\n' +
      '\r\n' +
      '\x1b[1;34mcapsem:~#\x1b[0m '
    ));

    terminal.onData((data: string) => {
      // WebSocket mode: send to gateway
      if (wsConnected && ws && ws.readyState === WebSocket.OPEN) {
        ws.send(data);
        return;
      }

      // Mock echo mode (no gateway)
      if (!mockEchoEnabled || !terminal) return;
      for (const ch of data) {
        if (ch === '\r') {
          terminal.write('\r\n\x1b[1;34mcapsem:~#\x1b[0m ');
        } else if (ch === '\x7f') {
          terminal.write('\b \b');
        } else if (ch === '\x03') {
          terminal.write('^C\r\n\x1b[1;34mcapsem:~#\x1b[0m ');
        } else {
          terminal.write(ch);
        }
      }
    });

    terminal.focus();

    // Tell parent we're ready -- triggers vm-id and theme-change
    sendToParent({ type: 'ready' });
  });

  onDestroy(() => {
    window.removeEventListener('message', onMessage);
    if (resizeRafId) cancelAnimationFrame(resizeRafId);
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
