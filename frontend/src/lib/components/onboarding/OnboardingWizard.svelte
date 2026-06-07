<script lang="ts">
  import { onMount } from 'svelte';
  import { onboardingStore } from '../../stores/onboarding.svelte.ts';
  import WelcomeStep from './WelcomeStep.svelte';
  import PreferencesStep from './PreferencesStep.svelte';
  import ProvidersStep from './ProvidersStep.svelte';
  import ReadyStep from './ReadyStep.svelte';

  const steps = ['Welcome', 'Preferences', 'Providers', 'Ready'];

  onMount(() => {
    onboardingStore.totalSteps = steps.length;
    onboardingStore.loadAssetStatus();
    return () => onboardingStore.destroy();
  });
</script>

<div class="fixed inset-0 z-50 flex flex-col bg-background">
  <!-- Progress bar -->
  <div class="flex items-center justify-center gap-2 px-8 pt-6 pb-2">
    {#each steps as label, i}
      <button
        type="button"
        class="flex items-center gap-1.5 text-xs font-medium transition-colors"
        class:text-primary={i === onboardingStore.currentStep}
        class:text-muted-foreground={i !== onboardingStore.currentStep}
        onclick={() => {
          if (i <= onboardingStore.currentStep) onboardingStore.goToStep(i);
        }}
        disabled={i > onboardingStore.currentStep}
      >
        <span
          class="size-6 rounded-full flex items-center justify-center text-xs font-semibold transition-colors"
          class:bg-primary={i <= onboardingStore.currentStep}
          class:text-primary-foreground={i <= onboardingStore.currentStep}
          class:bg-muted={i > onboardingStore.currentStep}
          class:text-muted-foreground={i > onboardingStore.currentStep}
        >
          {i + 1}
        </span>
        <span class="hidden sm:inline">{label}</span>
      </button>
      {#if i < steps.length - 1}
        <div class="w-8 h-px bg-line-2"></div>
      {/if}
    {/each}
  </div>

  <!-- Step content -->
  <div class="flex-1 overflow-y-auto px-8 py-6">
    <div class="max-w-lg mx-auto">
      {#if onboardingStore.currentStep === 0}
        <WelcomeStep />
      {:else if onboardingStore.currentStep === 1}
        <PreferencesStep />
      {:else if onboardingStore.currentStep === 2}
        <ProvidersStep />
      {:else if onboardingStore.currentStep === 3}
        <ReadyStep />
      {/if}
    </div>
  </div>

  <!-- Navigation -->
  <div class="flex items-center justify-between px-8 py-4 border-t border-line-2">
    <button
      type="button"
      class="py-2 px-4 text-sm font-medium rounded-lg bg-layer border border-layer-line text-layer-foreground hover:bg-layer-hover transition-colors"
      class:invisible={onboardingStore.currentStep === 0}
      onclick={() => onboardingStore.prevStep()}
    >
      Back
    </button>

    <div class="flex items-center gap-3">
      {#if onboardingStore.currentStep < steps.length - 1}
        <button
          type="button"
          class="py-2 px-4 text-sm text-muted-foreground hover:text-foreground transition-colors"
          onclick={() => onboardingStore.nextStep()}
        >
          Skip
        </button>
        <button
          type="button"
          class="py-2 px-4 text-sm font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover transition-colors"
          onclick={() => onboardingStore.nextStep()}
        >
          Next
        </button>
      {:else}
        <button
          type="button"
          class="py-2 px-6 text-sm font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover transition-colors"
          onclick={() => onboardingStore.completeOnboarding()}
        >
          Get Started
        </button>
      {/if}
    </div>
  </div>
</div>
