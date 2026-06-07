<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { parseIframeMessage } from '../../terminal/postmessage.ts';
  import type { ParentToIframeMsg } from '../../terminal/postmessage.ts';
  import { themeStore } from '../../stores/theme.svelte.ts';
  import { tabStore } from '../../stores/tabs.svelte.ts';

  let { vmId, tabId }: { vmId: string; tabId: string } = $props();

  let iframeRef: HTMLIFrameElement | null = $state(null);

  // allow-same-origin required so the iframe can fetch the gateway token
  // (tauri://localhost origin is CORS-whitelisted by the gateway) and open
  // a WebSocket without an opaque origin. Isolation comes from the sandbox
  // attribute + Tauri protocol boundary.
  const sandboxAttr = 'allow-scripts allow-same-origin';

  // Initial state baked into the iframe URL. vmId is the tab's identity, so
  // this is computed once per tab -- if vmId changes, the tab is a different
  // tab (keyed by tab.id in the parent #each), and the iframe remounts.
  const src = buildSrc();

  function buildSrc(): string {
    const p = new URLSearchParams();
    p.set('vm', vmId);
    p.set('mode', themeStore.mode);
    p.set('theme', themeStore.resolvedTerminalTheme);
    p.set('fontSize', String(themeStore.fontSize));
    if (themeStore.fontFamily) p.set('fontFamily', themeStore.fontFamily);
    // Explicit index.html -- Tauri v2 custom protocol on macOS does not
    // auto-append index.html for trailing-slash paths. Dev server (Astro/Vite)
    // does, which is why this worked in Chrome but not in Tauri.
    return `/vm/terminal/index.html?${p.toString()}`;
  }

  function sendToIframe(msg: ParentToIframeMsg): void {
    iframeRef?.contentWindow?.postMessage(msg, '*');
  }

  // Runtime theme changes flow to the iframe (fire-and-forget; if iframe isn't
  // mounted yet, it already has the current state from its URL params).
  $effect(() => {
    sendToIframe({
      type: 'theme-change',
      mode: themeStore.mode,
      terminalTheme: themeStore.resolvedTerminalTheme,
      fontSize: themeStore.fontSize,
      fontFamily: themeStore.fontFamily,
    });
  });

  // Focus the terminal whenever this tab becomes active.
  $effect(() => {
    if (tabStore.activeId === tabId) {
      requestAnimationFrame(() => sendToIframe({ type: 'focus' }));
    }
  });

  function onMessage(event: MessageEvent): void {
    if (event.source !== iframeRef?.contentWindow) return;
    const msg = parseIframeMessage(event.data);
    if (!msg) return;

    switch (msg.type) {
      case 'title-update':
        tabStore.updateSubtitle(tabId, msg.title);
        break;
      case 'clipboard-copy':
        navigator.clipboard.writeText(msg.text).catch(() => {});
        break;
      case 'clipboard-request':
        navigator.clipboard.readText()
          .then(text => sendToIframe({ type: 'clipboard-paste', text }))
          .catch(() => {});
        break;
      case 'connected':
      case 'disconnected':
      case 'error':
        // Status signals -- we could surface these in the tab UI later.
        break;
    }
  }

  onMount(() => {
    window.addEventListener('message', onMessage);
  });

  onDestroy(() => {
    window.removeEventListener('message', onMessage);
  });
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="w-full h-full" role="presentation" onclick={() => sendToIframe({ type: 'focus' })}>
  <iframe
    bind:this={iframeRef}
    sandbox={sandboxAttr}
    {src}
    title="Terminal: {vmId}"
    referrerpolicy="no-referrer"
    class="w-full h-full border-0"
  ></iframe>
</div>
