<script lang="ts">
  import { onMount } from 'svelte';
  import { onboardingStore } from '../../stores/onboarding.svelte.ts';

  onMount(() => {
    onboardingStore.loadAssetStatus();
  });

  function assetSummaryLabel(): string {
    if (onboardingStore.serviceStatus !== 'running') return 'Service offline';
    if (onboardingStore.assetsReady) {
      return onboardingStore.assetsVersion ? `Ready (v${onboardingStore.assetsVersion})` : 'Ready';
    }
    if (onboardingStore.assetsState === 'checking') return 'Checking';
    if (onboardingStore.assetsState === 'updating') return 'Updating';
    if (onboardingStore.assetsState === 'error') return 'Error';
    return 'Unknown';
  }

  function profileSummaryLabel(): string | null {
    if (onboardingStore.assetsProfileId) {
      return onboardingStore.assetsProfileRevision
        ? `${onboardingStore.assetsProfileId}@${onboardingStore.assetsProfileRevision}`
        : onboardingStore.assetsProfileId;
    }
    return onboardingStore.setupState?.security_preset ?? null;
  }
</script>

<div class="text-center space-y-6">
  <div class="size-16 mx-auto rounded-2xl bg-primary/10 flex items-center justify-center">
    <svg class="size-8 text-primary" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
      <path d="M5 13l4 4L19 7" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  </div>

  <div>
    <h2 class="text-xl font-medium text-foreground">Ready to Go</h2>
    <p class="mt-2 text-sm text-muted-foreground-1">
      Setup is complete. You can always reconfigure from Settings.
    </p>
  </div>

  <!-- Summary -->
  <div class="bg-card border border-card-line rounded-xl p-4 text-left space-y-3">
    <!-- Assets -->
    <div class="flex items-center justify-between text-sm">
      <span class="text-muted-foreground-1">VM Assets</span>
      <span class={onboardingStore.assetsReady ? 'text-primary' : 'text-muted-foreground'}>{assetSummaryLabel()}</span>
    </div>

    <!-- Providers -->
    {#if onboardingStore.detected}
      <div class="flex items-center justify-between text-sm">
        <span class="text-muted-foreground-1">AI Providers</span>
        <span class="text-foreground">
          {#if onboardingStore.detected.anthropic_api_key_present}Anthropic{/if}
          {#if onboardingStore.detected.openai_api_key_present}{onboardingStore.detected.anthropic_api_key_present ? ', ' : ''}OpenAI{/if}
          {#if onboardingStore.detected.google_api_key_present}{(onboardingStore.detected.anthropic_api_key_present || onboardingStore.detected.openai_api_key_present) ? ', ' : ''}Google{/if}
          {#if !onboardingStore.detected.anthropic_api_key_present && !onboardingStore.detected.openai_api_key_present && !onboardingStore.detected.google_api_key_present}
            None configured
          {/if}
        </span>
      </div>
    {/if}

    <!-- Profile state -->
    {#if profileSummaryLabel()}
      <div class="flex items-center justify-between text-sm">
        <span class="text-muted-foreground-1">Profile</span>
        <span class="text-foreground font-mono">{profileSummaryLabel()}</span>
      </div>
    {/if}
  </div>

  {#if !onboardingStore.assetsReady}
    <div class="rounded-lg border border-warning/30 bg-warning/10 p-3 text-left">
      <p class="text-xs text-foreground">
        VMs stay blocked until readiness is complete. You can still explore the app.
      </p>
      {#if onboardingStore.assetsState === 'updating'}
        <p class="mt-1 text-xs text-muted-foreground">Assets are downloading in the background.</p>
      {:else if onboardingStore.assetsState === 'checking'}
        <p class="mt-1 text-xs text-muted-foreground">The service is verifying required assets.</p>
      {:else if onboardingStore.assetsState === 'error'}
        <p class="mt-1 text-xs text-muted-foreground">{onboardingStore.assetsError ?? 'Asset preparation failed.'}</p>
      {:else if onboardingStore.serviceStatus !== 'running'}
        <p class="mt-1 text-xs text-muted-foreground">Start the service to continue readiness checks.</p>
      {/if}
    </div>
  {/if}

  {#if onboardingStore.savedVmDependencies.length > 0}
    <div class="rounded-lg border border-warning/30 bg-warning/10 p-3 text-left">
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
</div>
