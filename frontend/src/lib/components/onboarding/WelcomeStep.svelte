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

  type AssetCardState = {
    title: string;
    message: string;
    showMissing: boolean;
    showRetry: boolean;
  };

  function assetCardState(): AssetCardState {
    if (onboardingStore.serviceStatus !== 'running') {
      return {
        title: 'Capsem service is offline',
        message: 'Start the service so setup can verify VM readiness.',
        showMissing: false,
        showRetry: false,
      };
    }

    if (onboardingStore.assetsReady) {
      return {
        title: 'Assets ready',
        message: `All required VM assets are ready${onboardingStore.assetsVersion ? ` (v${onboardingStore.assetsVersion})` : ''}.`,
        showMissing: false,
        showRetry: false,
      };
    }

    if (onboardingStore.assetsState === 'checking') {
      return {
        title: 'Checking VM assets',
        message: 'The service is verifying required VM assets.',
        showMissing: false,
        showRetry: false,
      };
    }

    if (onboardingStore.assetsState === 'updating') {
      return {
        title: 'Updating VM assets',
        message: onboardingStore.assetsProgressLabel
          ? `Downloading ${onboardingStore.assetsProgressLabel}.`
          : 'Required VM assets are downloading in the background.',
        showMissing: onboardingStore.assetsMissing.length > 0,
        showRetry: false,
      };
    }

    if (onboardingStore.assetsState === 'error') {
      return {
        title: 'VM assets need attention',
        message: onboardingStore.assetsError ?? 'The service could not prepare required VM assets.',
        showMissing: onboardingStore.assetsMissing.length > 0,
        showRetry: onboardingStore.assetsRetryable,
      };
    }

    return {
      title: 'VM asset status is unknown',
      message: 'Waiting for the service to report asset readiness.',
      showMissing: onboardingStore.assetsMissing.length > 0,
      showRetry: false,
    };
  }

  let card = $derived.by(() => assetCardState());

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
    <p class="text-sm text-foreground">{card.title}</p>
    <p class="text-xs text-muted-foreground mt-1">{card.message}</p>

    {#if card.showMissing}
      <div class="space-y-2 mt-3">
        {#each onboardingStore.assetsMissing as name}
          <div class="flex items-center justify-between text-sm">
            <span class="text-foreground font-mono text-xs">{name}</span>
            <span class="text-muted-foreground text-xs">Missing</span>
          </div>
        {/each}
      </div>
    {/if}

    {#if onboardingStore.savedVmDependencies.length > 0}
      <div class="mt-3 rounded-lg border border-warning/30 bg-warning/10 p-3">
        <p class="text-xs font-medium text-foreground">Saved VM dependencies missing</p>
        <ul class="mt-2 space-y-1">
          {#each onboardingStore.savedVmDependencies as dep}
            <li class="text-xs text-muted-foreground">
              <span class="font-mono text-foreground">{dep.vm}</span> missing {dep.missing.join(', ')} ({dep.recovery_hint})
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    <div class="mt-3 flex items-center gap-2">
      {#if card.showRetry}
        <button
          type="button"
          class="py-1 px-3 text-xs font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          disabled={onboardingStore.retrying}
          onclick={() => onboardingStore.retryInstall()}
        >
          {onboardingStore.retrying ? 'Retrying...' : 'Retry setup'}
        </button>
      {/if}
      <button
        type="button"
        class="py-1 px-3 text-xs font-medium rounded-lg bg-layer border border-layer-line text-layer-foreground hover:bg-layer-hover transition-colors"
        onclick={() => onboardingStore.loadAssetStatus()}
      >
        Refresh status
      </button>
    </div>
    {#if onboardingStore.retryError}
      <p class="mt-2 text-xs text-destructive">{onboardingStore.retryError}</p>
    {/if}
  </div>
</div>
