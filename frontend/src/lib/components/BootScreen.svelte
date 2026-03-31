<script lang="ts">
  import { vmStore } from '../stores/vm.svelte';
  import { wizardStore } from '../stores/wizard.svelte';
  import { sidebarStore } from '../stores/sidebar.svelte';
  import { html as releaseHtml, version as releaseVersion } from 'virtual:release-notes';

  const downloading = $derived(vmStore.isDownloading);
  const booting = $derived(vmStore.isBooting);
  const errored = $derived(vmStore.isError);
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

  const ERROR_HINTS: Record<string, string> = {
    assets_not_found: 'VM assets not found. Run "just build-assets" to build them.',
    asset_init_failed: 'Could not initialize the asset manager. Check that manifest.json exists in the assets directory.',
    manifest_error: 'The rootfs entry is missing from the asset manifest. Try rebuilding with "just build-assets".',
    download_failed: 'Failed to download the VM image. Check your internet connection and try restarting.',
  };
</script>

<div class="flex items-center justify-center h-full bg-base-100">
  <div class="flex flex-col items-center gap-6 max-w-2xl w-full px-8">

    <!-- Header -->
    <div class="text-center space-y-1">
      <h1 class="text-3xl font-semibold text-base-content">Capsem</h1>
      {#if releaseVersion}
        <p class="text-sm text-base-content/50">
          {#if errored}
            v{releaseVersion}
          {:else if downloading}
            Updating to v{releaseVersion}...
          {:else if ready}
            v{releaseVersion} ready
          {:else}
            Starting v{releaseVersion}...
          {/if}
        </p>
      {/if}
    </div>

    <!-- Error state -->
    {#if errored}
      <div class="w-full max-w-md space-y-3">
        <div class="alert border border-denied/30 bg-denied/10">
          <svg xmlns="http://www.w3.org/2000/svg" class="h-5 w-5 shrink-0 text-denied" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L4.082 16.5c-.77.833.192 2.5 1.732 2.5z" />
          </svg>
          <div class="space-y-1">
            <p class="text-sm font-medium text-denied">Failed to start sandbox</p>
            {#if vmStore.errorTrigger && ERROR_HINTS[vmStore.errorTrigger]}
              <p class="text-xs text-base-content/60">{ERROR_HINTS[vmStore.errorTrigger]}</p>
            {:else if vmStore.errorMessage}
              <p class="text-xs text-base-content/60">{vmStore.errorMessage}</p>
            {:else}
              <p class="text-xs text-base-content/60">An unexpected error occurred. Check the logs for details.</p>
            {/if}
          </div>
        </div>
      </div>

    <!-- Download progress -->
    {:else if downloading}
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

    <!-- Booting -->
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
        {#if errored}
          Sandbox unavailable
        {:else if downloading}
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
