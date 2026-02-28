<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import type { CapsemTerminal } from '../../components/capsem-terminal';
  import { serialInput, terminalResize, terminalPoll, onTerminalSourceChanged } from '../api';
  import { isMock } from '../mock';
  import { themeStore } from '../stores/theme.svelte';
  import { vmStore } from '../stores/vm.svelte';

  // Side-effect: register the web component
  import '../../components/capsem-terminal';

  let termEl: CapsemTerminal;
  let cleanups: (() => void)[] = [];
  let mounted = true;

  // Input batching: accumulate keystrokes for 5ms before sending.
  const INPUT_BATCH_MS = 5;
  const INPUT_BATCH_MAX = 4096;
  let inputBuffer = '';
  let inputTimer: ReturnType<typeof setTimeout> | null = null;

  function flushInput() {
    if (inputTimer !== null) {
      clearTimeout(inputTimer);
      inputTimer = null;
    }
    if (inputBuffer.length === 0) return;
    const batch = inputBuffer;
    inputBuffer = '';
    serialInput(batch).catch(() => {});
  }

  // React to theme changes via $effect
  $effect(() => {
    const t = themeStore.theme;
    if (termEl) {
      termEl.setTheme(t);
    }
  });

  onMount(async () => {
    if (!termEl) return;

    // Set initial theme
    termEl.setTheme(themeStore.theme);

    // Forward terminal input to Tauri with batching
    const onInput = ((e: CustomEvent) => {
      inputBuffer += e.detail;
      if (inputBuffer.length >= INPUT_BATCH_MAX) {
        flushInput();
      } else if (inputTimer === null) {
        inputTimer = setTimeout(flushInput, INPUT_BATCH_MS);
      }
    }) as EventListener;
    termEl.addEventListener('terminal-input', onInput);
    cleanups.push(() => termEl.removeEventListener('terminal-input', onInput));

    // Forward terminal resize to Tauri
    const onResize = ((e: CustomEvent) => {
      const { cols, rows } = e.detail;
      terminalResize(cols, rows).catch(() => {});
    }) as EventListener;
    termEl.addEventListener('terminal-resize', onResize);
    cleanups.push(() => termEl.removeEventListener('terminal-resize', onResize));

    // Poll-based output loop. Yields to the browser via rAF after each write
    // so xterm.js rendering and UI events aren't starved by a tight loop.
    if (!isMock) {
      (async function pollTerminalOutput() {
        while (mounted) {
          try {
            const data = await terminalPoll();
            if (data.length > 0) {
              termEl.write(new Uint8Array(data));
              // Yield to the browser render cycle so xterm can paint and
              // UI events (input, resize) are processed between writes.
              await new Promise(r => requestAnimationFrame(r));
            }
          } catch (e) {
            if (String(e) === 'terminal closed') break;
            // Back off on errors to avoid tight error loop.
            await new Promise(r => setTimeout(r, 100));
          }
        }
      })();
    }

    // When vsock connects, re-fit the terminal and send the resize to the
    // guest. The initial fit() in connectedCallback fires before vsock is
    // ready, so the resize is silently dropped.
    const unSource = await onTerminalSourceChanged((_source) => {
      termEl.fit();
    });
    cleanups.push(unSource);

    // Also watch for data-theme changes on <html> as a fallback
    const observer = new MutationObserver(() => {
      const theme = document.documentElement.getAttribute('data-theme');
      if (theme === 'light' || theme === 'dark') {
        termEl.setTheme(theme);
      }
    });
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    cleanups.push(() => observer.disconnect());

    // In mock mode, write a demo banner
    if (isMock) {
      const encoder = new TextEncoder();
      termEl.write(encoder.encode(
        '\x1b[1;34mCAPSEM sandbox ready\x1b[0m\r\n' +
        '\x1b[35mLinux 6.6.127 | aarch64\x1b[0m\r\n' +
        '\r\n' +
        'Dev:  python3  node  npm  git  vim\r\n' +
        'AI:   claude   gemini  codex\r\n' +
        'Test: capsem-test\r\n' +
        '\r\n' +
        '\x1b[1;34mcapsem:~#\x1b[0m '
      ));
    }

    vmStore.terminalRenderer = termEl.renderer;
    termEl.focusTerminal();
  });

  onDestroy(() => {
    mounted = false;
    flushInput();
    for (const fn of cleanups) fn();
  });

  export function focus() {
    termEl?.focusTerminal();
  }
</script>

<capsem-terminal bind:this={termEl} class="block h-full w-full"></capsem-terminal>
