<script lang="ts">
  import { vmStore } from '../stores/vm.svelte';

  let progress = $derived(vmStore.downloadProgress);
  let pct = $derived(
    progress && progress.total_bytes > 0
      ? Math.round((progress.bytes_downloaded / progress.total_bytes) * 100)
      : 0,
  );
  let downloaded = $derived(
    progress ? formatBytes(progress.bytes_downloaded) : '0 B',
  );
  let total = $derived(
    progress && progress.total_bytes > 0 ? formatBytes(progress.total_bytes) : '...',
  );

  function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
</script>

<div class="flex items-center justify-center h-full bg-base-100">
  <div class="flex flex-col items-center gap-6 max-w-md w-full px-8">
    <h2 class="text-xl font-semibold text-base-content">Downloading VM image</h2>
    <p class="text-sm text-base-content/60 text-center">
      The sandbox rootfs is downloaded once and cached locally.
      {#if progress?.phase === 'verifying'}
        Verifying integrity...
      {:else if progress?.phase === 'connecting'}
        Connecting...
      {/if}
    </p>

    <div class="w-full">
      <progress
        class="progress w-full h-3 [&::-webkit-progress-value]:bg-allowed [&::-moz-progress-bar]:bg-allowed"
        value={pct}
        max="100"
      ></progress>
    </div>

    <div class="flex justify-between w-full text-xs text-base-content/50">
      <span>{downloaded} / {total}</span>
      <span>{pct}%</span>
    </div>
  </div>
</div>
