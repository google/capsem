<script lang="ts">
  import { onMount } from 'svelte';
  import { onboardingStore } from '../../stores/onboarding.svelte.ts';
  import welcomeRaw from './welcome.md?raw';

  // Parse the markdown into simple HTML (headings + list items)
  function parseWelcomeMd(raw: string): { heading: string; items: string[] } {
    const lines = raw.trim().split('\n').filter(l => l.trim());
    const heading = lines[0]?.replace(/^#+\s*/, '') ?? "What's New";
    const items = lines.slice(1)
      .filter(l => l.startsWith('- '))
      .map(l => l.replace(/^- /, '').replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>'));
    return { heading, items };
  }

  const whatsNew = parseWelcomeMd(welcomeRaw);

  onMount(() => {
    onboardingStore.loadAssetStatus();
  });
</script>

<div class="text-center space-y-6">
  <img src="/logo-color.png" alt="Capsem" class="size-16 mx-auto" />

  <div>
    <h1 class="text-2xl font-semibold text-foreground">Welcome to Capsem</h1>
    <p class="mt-2 text-sm text-muted-foreground-1">
      The fastest way to ship with AI securely.
    </p>
  </div>

  <!-- What's New -->
  <div class="bg-card border border-card-line rounded-xl p-4 text-left">
    <h3 class="text-sm font-medium text-foreground mb-3">{whatsNew.heading}</h3>
    <ul class="space-y-2">
      {#each whatsNew.items as item}
        <li class="text-sm text-muted-foreground-1">{@html item}</li>
      {/each}
    </ul>
  </div>

  <!-- Asset status -->
  <div class="bg-card border border-card-line rounded-xl p-4 text-left">
    <h3 class="text-sm font-medium text-foreground mb-3">VM Assets</h3>

    {#if onboardingStore.assetsReady}
      <div class="flex items-center gap-2 text-sm text-primary">
        <svg class="size-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M5 13l4 4L19 7" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
        All assets ready{#if onboardingStore.assetsVersion}&nbsp;(v{onboardingStore.assetsVersion}){/if}.
      </div>
    {:else if onboardingStore.assetsMissing.length > 0}
      <div class="space-y-2">
        {#each onboardingStore.assetsMissing as name}
          <div class="flex items-center justify-between text-sm">
            <span class="text-foreground font-mono text-xs">{name}</span>
            <span class="text-muted-foreground text-xs">Missing</span>
          </div>
        {/each}
        <p class="text-xs text-muted-foreground mt-2">
          Run <code class="px-1 py-0.5 rounded bg-background-1 text-xs">capsem update</code> in the terminal to download assets.
        </p>
      </div>
    {:else}
      <p class="text-sm text-muted-foreground-1">Checking asset status...</p>
    {/if}
  </div>
</div>
