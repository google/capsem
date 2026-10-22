<script lang="ts">
  import AgentSetup from "./AgentSetup.svelte";
  import Modal from "./Modal.svelte";
  import { onMount } from "svelte";
  import { downloadIndex } from "$lib/download";
  import { DOWNLOAD as D, SITE, UI } from "$lib/data";
  let packages = $state(D.packages.map(item => ({ ...item, url: String(item.url) })));
  let selected = $state<number | null>(null);
  let open = $state(false);
  const apple = "M17.1 12.3c0-2 1.6-3 1.7-3.1-1-1.5-2.5-1.7-3-1.7-1.3-.1-2.5.8-3.1.8-.7 0-1.7-.8-2.8-.8-1.4 0-2.8.9-3.5 2.1-1.5 2.6-.4 6.5 1 8.6.7 1 1.5 2.1 2.6 2.1 1 0 1.5-.7 2.8-.7s1.7.7 2.9.7 1.9-1 2.6-2c.8-1.2 1.1-2.3 1.1-2.4-.1 0-2.3-.9-2.3-3.6ZM15 6.2c.6-.8 1-1.8.9-2.8-.9 0-2 .6-2.7 1.3-.6.7-1.1 1.7-1 2.7 1 .1 2.1-.5 2.8-1.2Z";
  onMount(() => {
    const ua = navigator.userAgent;
    selected = downloadIndex(ua);
    const hints = (navigator as Navigator & { userAgentData?: { getHighEntropyValues(keys: string[]): Promise<{ architecture?: string }> } }).userAgentData;
    if (selected === 1 && hints) void hints.getHighEntropyValues(["architecture"]).then(value => {
      selected = downloadIndex(ua, value.architecture);
    }).catch(() => {});
    const controller = new AbortController();
    void fetch(D.manifest, { signal: controller.signal }).then(response => {
      if (!response.ok) throw new Error("Release manifest unavailable");
      return response.json();
    }).then((manifest: { packages?: { platform: string; architecture: string; status: string; url: string }[] }) => {
      if (!Array.isArray(manifest.packages)) return;
      packages = packages.map(item => {
        const release = manifest.packages?.find(candidate => candidate.status === "current" && candidate.platform === item.platform && candidate.architecture === item.architecture);
        return release && /^https:\/\/github\.com\/google\/capsem\/releases\/download\/[^/]+\/[^/]+\.(pkg|dmg|deb)$/.test(release.url) ? { ...item, url: release.url } : item;
      });
    }).catch(() => { /* The verified stable links remain available offline. */ });
    return () => controller.abort();
  });
</script>

<div class="flex flex-wrap items-center gap-3">
  <div class="inline-flex shrink-0 items-stretch rounded-full bg-linear-to-r from-accent to-secondary text-white">
  {#if selected !== null}
    <a class="inline-flex items-center justify-center gap-2 rounded-l-full px-5 py-3 text-sm font-semibold hover:bg-white/10" href={packages[selected].url}>
      {#if selected === 0}<svg class="size-6" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d={apple}/></svg>{:else}<span class="font-mono" aria-hidden="true">&gt;_</span>{/if}
      {UI.download}
    </a>
  {:else}
    <button class="inline-flex items-center justify-center gap-2 rounded-l-full px-5 py-3 text-sm font-semibold hover:bg-white/10" onclick={() => open = true}>{UI.download}<span aria-hidden="true">↓</span></button>
  {/if}
  <button type="button" class="grid w-9 place-items-center rounded-r-full border-l border-white/25 hover:bg-white/10" aria-label={D.other} title={D.other} onclick={() => open = true}><svg class="size-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><path d="m4 6 4 4 4-4" /></svg></button>
  </div>
  <AgentSetup />
</div>
<Modal bind:open title={D.title} closeLabel={D.close}>
  <p class="mb-4 text-sm text-body">{D.choose}</p>
  <div class="divide-y divide-line">{#each packages as item}<a href={item.url} class="flex items-center justify-between gap-4 py-4 hover:text-accent-text"><span><strong class="block text-base font-semibold">{item.label}</strong><span class="text-sm text-muted">{item.detail}</span></span><span aria-hidden="true">↓</span></a>{/each}</div>
  <a class="mt-4 inline-block text-sm text-accent-text underline underline-offset-4" href={SITE.releases}>Release details ↗</a>
</Modal>
