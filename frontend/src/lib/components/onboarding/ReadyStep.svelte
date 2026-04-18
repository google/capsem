<script lang="ts">
  import { onMount } from 'svelte';
  import { onboardingStore } from '../../stores/onboarding.svelte.ts';

  onMount(() => {
    onboardingStore.loadAssetStatus();
  });
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
      {#if onboardingStore.assetsReady}
        <span class="text-primary">Ready{#if onboardingStore.assetsVersion}&nbsp;(v{onboardingStore.assetsVersion}){/if}</span>
      {:else}
        <span class="text-muted-foreground">Not downloaded</span>
      {/if}
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

    <!-- Setup state -->
    {#if onboardingStore.setupState?.security_preset}
      <div class="flex items-center justify-between text-sm">
        <span class="text-muted-foreground-1">Security Preset</span>
        <span class="text-foreground capitalize">{onboardingStore.setupState.security_preset}</span>
      </div>
    {/if}
  </div>

  {#if !onboardingStore.assetsReady}
    <p class="text-xs text-muted-foreground">
      VMs won't boot until assets are downloaded. You can still explore the app.
    </p>
  {/if}
</div>
