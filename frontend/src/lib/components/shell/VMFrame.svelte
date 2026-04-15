<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { parseIframeMessage } from '../../terminal/postmessage.ts';
  import type { ParentToIframeMsg } from '../../terminal/postmessage.ts';
  import { themeStore } from '../../stores/theme.svelte.ts';
  import { tabStore } from '../../stores/tabs.svelte.ts';
  import { gatewayStore } from '../../stores/gateway.svelte.ts';
  import * as api from '../../api';

  let { vmId, tabId }: { vmId: string; tabId: string } = $props();

  let iframeRef: HTMLIFrameElement | null = $state(null);

  // SECURITY: In production, sandbox MUST be "allow-scripts" only.
  // "allow-same-origin" is added in dev because Vite's module scripts
  // require same-origin access. The production static build serves from
  // the same origin so CORS is not an issue.
  // INVARIANT: Never ship allow-same-origin. It collapses the sandbox.
  const DEV = (import.meta as any).env?.DEV ?? false;
  const sandboxAttr = DEV ? 'allow-scripts allow-same-origin' : 'allow-scripts';

  function sendToIframe(msg: ParentToIframeMsg): void {
    iframeRef?.contentWindow?.postMessage(msg, '*');
  }

  // Forward theme + font changes to iframe
  $effect(() => {
    const mode = themeStore.mode;
    const termTheme = themeStore.resolvedTerminalTheme;
    const fontSize = themeStore.fontSize;
    const fontFamily = themeStore.fontFamily;
    sendToIframe({ type: 'theme-change', mode, terminalTheme: termTheme, fontSize, fontFamily });
  });

  // Refocus terminal when this tab becomes active again.
  // Use requestAnimationFrame so the browser has painted the unhidden container first.
  $effect(() => {
    if (tabStore.activeId === tabId) {
      requestAnimationFrame(() => sendToIframe({ type: 'focus' }));
    }
  });

  function onMessage(event: MessageEvent): void {
    // Only accept messages from our iframe
    if (event.source !== iframeRef?.contentWindow) return;

    const msg = parseIframeMessage(event.data);
    if (!msg) return;

    switch (msg.type) {
      case 'ready':
        sendToIframe({ type: 'vm-id', vmId });
        sendToIframe({
          type: 'theme-change',
          mode: themeStore.mode,
          terminalTheme: themeStore.resolvedTerminalTheme,
          fontSize: themeStore.fontSize,
          fontFamily: themeStore.fontFamily,
        });
        // Send WebSocket ticket if gateway is connected
        if (gatewayStore.connected) {
          const wsUrl = api.getTerminalWsUrl(vmId);
          const base = api.getBaseUrl();
          sendToIframe({ type: 'ws-ticket', ticket: wsUrl, gatewayUrl: base });
        }
        break;

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

      case 'terminal-resize':
        // Resize events are handled by the iframe's WebSocket connection directly
        break;

      case 'error':
        console.warn(`VM ${vmId}: ${msg.code}: ${msg.message}`);
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
    src="/vm/terminal/"
    title="Terminal: {vmId}"
    referrerpolicy="no-referrer"
    class="w-full h-full border-0"
  ></iframe>
</div>
