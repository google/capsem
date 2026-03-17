<script lang="ts">
  import { onMount } from 'svelte';
  import { wizardStore } from '../../stores/wizard.svelte';
  import { settingsStore } from '../../stores/settings.svelte';
  import { vmStore } from '../../stores/vm.svelte';
  import WelcomeStep from './WelcomeStep.svelte';
  import SecurityStep from './SecurityStep.svelte';
  import ProvidersStep from './ProvidersStep.svelte';
  import RepositoriesStep from './RepositoriesStep.svelte';
  import McpStep from './McpStep.svelte';
  import AllSetStep from './AllSetStep.svelte';

  onMount(async () => {
    await wizardStore.loadHostConfig();
    // Reload settings tree so it reflects auto-applied values
    await settingsStore.load();
  });

  const progress = $derived(vmStore.downloadProgress);
  const pct = $derived(
    progress && progress.total_bytes > 0
      ? Math.round((progress.bytes_downloaded / progress.total_bytes) * 100)
      : 0,
  );
  const mbDown = $derived(progress ? Math.round(progress.bytes_downloaded / (1024 * 1024)) : 0);
  const mbTotal = $derived(progress ? Math.round(progress.total_bytes / (1024 * 1024)) : 0);
  // Show download bar whenever we have progress data that hasn't finished yet.
  // Also show while vmState is 'downloading' (before first progress event).
  const showDownloadBar = $derived(vmStore.isDownloading || (progress !== null && pct < 100));
</script>

<div class="flex h-full flex-col">
  <!-- Step indicator dots -->
  <div class="flex justify-center gap-2 py-4">
    {#each Array(wizardStore.totalSteps) as _, i}
      <div
        class="h-2 w-2 rounded-full transition-colors {i === wizardStore.currentStep
          ? 'bg-interactive'
          : i < wizardStore.currentStep
            ? 'bg-allowed'
            : 'bg-base-content/20'}"
      ></div>
    {/each}
  </div>

  <!-- Step content area -->
  <div class="flex-1 overflow-y-auto px-6">
    <div class="mx-auto max-w-2xl py-6">
      {#if wizardStore.stepId === 'welcome'}
        <WelcomeStep />
      {:else if wizardStore.stepId === 'security'}
        <SecurityStep />
      {:else if wizardStore.stepId === 'providers'}
        <ProvidersStep />
      {:else if wizardStore.stepId === 'repositories'}
        <RepositoriesStep />
      {:else if wizardStore.stepId === 'mcp'}
        <McpStep />
      {:else if wizardStore.stepId === 'allset'}
        <AllSetStep />
      {/if}
    </div>
  </div>

  <!-- Download progress bar (pinned bottom) -->
  {#if showDownloadBar}
    <div
      class="flex h-10 items-center gap-3 border-t border-base-300 bg-base-200 px-4 text-xs text-base-content/60"
    >
      <span>Downloading VM image</span>
      <progress class="progress w-48" value={pct} max="100"></progress>
      <span>{mbDown}/{mbTotal} MB {pct}%</span>
    </div>
  {/if}
</div>
