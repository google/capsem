<script lang="ts">
  import { wizardStore } from '../../stores/wizard.svelte';
  import { settingsStore } from '../../stores/settings.svelte';
  import { mcpStore } from '../../stores/mcp.svelte';
  import { vmStore } from '../../stores/vm.svelte';
  import { sidebarStore } from '../../stores/sidebar.svelte';

  const presetName = $derived(
    settingsStore.presets.find((p) => p.id === settingsStore.activePresetId)?.name ?? 'Custom',
  );

  const enabledProviders = $derived(
    ['ai.anthropic.allow', 'ai.google.allow', 'ai.openai.allow'].filter(
      (id) => settingsStore.findLeaf(id)?.effective_value === true,
    ).length,
  );

  const hasRepoTokens = $derived(
    ['repository.providers.github.token', 'repository.providers.gitlab.token'].some(
      (id) => {
        const v = settingsStore.findLeaf(id)?.effective_value;
        return typeof v === 'string' && v.trim().length > 0;
      },
    ),
  );

  const mcpCount = $derived(mcpStore.servers.filter((s) => !s.unsupported_stdio).length);

  const downloading = $derived(vmStore.isDownloading);

  function finish() {
    wizardStore.finish();
    sidebarStore.setView('terminal');
  }

  interface SummaryItem {
    label: string;
    value: string;
    configured: boolean;
  }

  const summary: SummaryItem[] = $derived([
    { label: 'Security preset', value: presetName, configured: settingsStore.activePresetId !== null },
    { label: 'AI providers', value: `${enabledProviders} enabled`, configured: enabledProviders > 0 },
    { label: 'Repository tokens', value: hasRepoTokens ? 'Configured' : 'None', configured: hasRepoTokens },
    { label: 'MCP servers', value: `${mcpCount}`, configured: mcpCount > 0 },
  ]);
</script>

<div class="space-y-6">
  <div>
    <h2 class="text-2xl font-semibold">All Set</h2>
    <p class="text-sm text-base-content/60 mt-1">
      Here's a summary of your configuration.
    </p>
  </div>

  <!-- Summary checklist -->
  <div class="card border border-base-300 p-4 space-y-3">
    {#each summary as item}
      <div class="flex items-center gap-3">
        {#if item.configured}
          <span class="text-allowed text-sm">&#10003;</span>
        {:else}
          <span class="text-base-content/30 text-sm">--</span>
        {/if}
        <span class="text-sm">{item.label}</span>
        <span class="text-xs text-base-content/50 ml-auto">{item.value}</span>
      </div>
    {/each}
  </div>

  <!-- Quick reference -->
  <div class="text-xs text-base-content/40 space-y-1">
    <p>Quick reference:</p>
    <p>Terminal icon = console &middot; Gear icon = settings</p>
  </div>

  <!-- Nav -->
  <div class="flex justify-between pt-4">
    <button class="btn btn-ghost btn-sm" onclick={() => wizardStore.back()}>Back</button>
    <button
      class="btn bg-interactive text-white btn-sm"
      disabled={downloading}
      onclick={finish}
    >
      {#if downloading}
        <span class="loading loading-spinner loading-xs"></span>
        Finishing up download...
      {:else}
        Let's Go
      {/if}
    </button>
  </div>
</div>
