<script lang="ts">
  import { onDestroy } from "svelte";
  import { SITE, UI } from "$lib/data";
  let message = $state<string>(UI.copy);
  let timer: ReturnType<typeof setTimeout> | undefined;
  onDestroy(() => clearTimeout(timer));
  async function copy() {
    try { await navigator.clipboard.writeText(SITE.installCmd); message = UI.copied; }
    catch { message = UI.copyFailed; }
    clearTimeout(timer);
    timer = setTimeout(() => { message = UI.copy; }, 2500);
  }
</script>
<div class="install-command">
  <span aria-hidden="true">$</span><code>{SITE.installCmd}</code>
  <button onclick={copy} aria-label={UI.copy} title={message}>
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><rect x="8" y="8" width="12" height="12" rx="2"/><path d="M15 8V4H4v11h4"/></svg>
  </button>
  <span class="sr-only" role="status">{message === UI.copy ? "" : message}</span>
</div>
