<script lang="ts">
  import { vmStore } from '../stores/vm.svelte';
  import { wizardStore } from '../stores/wizard.svelte';
  import { sidebarStore } from '../stores/sidebar.svelte';
  import { html as releaseHtml, version as releaseVersion } from 'virtual:release-notes';

  const downloading = $derived(vmStore.isDownloading);
  const ready = $derived(vmStore.isRunning);

  let progress = $derived(vmStore.downloadProgress);
  let pct = $derived(
    progress && progress.total_bytes > 0
      ? Math.round((progress.bytes_downloaded / progress.total_bytes) * 100)
      : 0,
  );

  function formatMB(bytes: number): string {
    return `${Math.round(bytes / (1024 * 1024))} MB`;
  }

  function go() {
    sidebarStore.setView('terminal');
  }
</script>

<div class="flex items-center justify-center h-full bg-base-100">
  <div class="flex flex-col items-center gap-6 max-w-2xl w-full px-8">

    <!-- Header -->
    <div class="text-center space-y-1">
      <h1 class="text-3xl font-semibold text-base-content">Capsem</h1>
      {#if releaseVersion}
        <p class="text-sm text-base-content/50">
          {#if downloading}
            Updating to v{releaseVersion}...
          {:else if ready}
            v{releaseVersion} ready
          {:else}
            Starting v{releaseVersion}...
          {/if}
        </p>
      {/if}
    </div>

    <!-- Download progress -->
    {#if downloading}
      <div class="w-full max-w-md space-y-1">
        <progress
          class="progress progress-info w-full"
          value={pct}
          max="100"
        ></progress>
        <p class="text-xs text-base-content/40 text-center">
          {#if progress}
            {formatMB(progress.bytes_downloaded)} / {formatMB(progress.total_bytes)}
            &middot; {pct}%
          {:else}
            Connecting...
          {/if}
        </p>
      </div>
    {:else if !ready}
      <div class="flex items-center gap-2 text-sm text-base-content/50">
        <span class="loading loading-spinner loading-sm"></span>
        Booting sandbox...
      </div>
    {/if}

    <!-- Release notes -->
    <div class="card card-bordered bg-base-200/50 w-full">
      <div class="card-body p-4 max-h-72 overflow-y-auto">
        <h3 class="card-title text-sm text-base-content/60">What's new</h3>
        <div
          class="prose prose-sm max-w-none
                 prose-headings:text-base-content/60 prose-headings:text-sm prose-headings:font-normal
                 prose-p:text-base-content/60 prose-li:text-base-content/60
                 prose-strong:text-base-content/70 prose-strong:font-normal
                 prose-a:text-interactive [&_strong]:!font-normal"
        >
          {@html releaseHtml}
        </div>
      </div>
    </div>

    <!-- Actions -->
    <div class="flex items-center gap-3">
      <button class="btn btn-outline btn-sm" onclick={() => wizardStore.rerun()}>
        Re-run Setup Wizard
      </button>
      <button
        class="btn bg-interactive text-white btn-sm"
        disabled={!ready}
        onclick={go}
      >
        {#if downloading}
          <span class="loading loading-spinner loading-xs"></span>
          Downloading...
        {:else if !ready}
          <span class="loading loading-spinner loading-xs"></span>
          Starting...
        {:else}
          Let's Go
        {/if}
      </button>
    </div>

  </div>
</div>
