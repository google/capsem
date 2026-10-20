<script lang="ts">
  import { vmStore } from '../../stores/vms.svelte.ts';
  import { tabStore } from '../../stores/tabs.svelte.ts';
  import Modal from './Modal.svelte';
  import { ImageCacheState, type ImageInfo } from '@capsem/sdk';
  import { getImages } from '../../api';
  import { sessionCreateRequest } from '../../models/session-create';
  import { sessionActionError } from '../../models/session-action-error';

  // Mirrors the service's defaults for a session created without explicit resources.
  const DEFAULT_RAM_MB = 12288;
  const DEFAULT_CPUS = 4;

  let name = $state('');
  let ramMb = $state(DEFAULT_RAM_MB);
  let cpus = $state(DEFAULT_CPUS);
  let error = $state<string | null>(null);
  let creating = $state(false);
  let images = $state<ImageInfo[]>([]);
  let imageName = $state('');
  let imagesLoading = $state(false);
  let imagesError = $state<string | null>(null);
  let selectedImage = $derived(images.find(image => image.name === imageName) ?? null);

  $effect(() => {
    if (!vmStore.showCreateModal) return;
    const controller = new AbortController();
    imagesLoading = true;
    imagesError = null;
    void getImages(controller.signal).then(result => {
      if (!controller.signal.aborted) images = result.images;
    }).catch((e: unknown) => {
      if (!controller.signal.aborted) imagesError = sessionActionError(e);
    }).finally(() => {
      if (!controller.signal.aborted) imagesLoading = false;
    });
    return () => controller.abort();
  });

  function close() {
    if (creating) return;
    vmStore.closeCreateModal();
    name = '';
    ramMb = DEFAULT_RAM_MB;
    cpus = DEFAULT_CPUS;
    error = null;
    images = [];
    imageName = '';
    imagesError = null;
  }

  async function handleSubmit() {
    if (creating) return;
    error = null;
    const trimmedName = name.trim();
    if (!trimmedName) {
      error = 'Name is required';
      return;
    }
    creating = true;
    try {
      const { id, name: finalName } = await vmStore.provision(sessionCreateRequest(trimmedName, ramMb, cpus, selectedImage));
      tabStore.openVM(id, finalName);
      creating = false;
      close();
    } catch (e: unknown) {
      error = sessionActionError(e);
    } finally {
      creating = false;
    }
  }
</script>

<Modal
  open={vmStore.showCreateModal}
  title="Customize session"
  confirmLabel={creating ? 'Creating...' : 'Create'}
  onconfirm={handleSubmit}
  oncancel={close}
  disabled={creating}
>
  <div class="space-y-4 py-2">
    {#if error}
      <div role="alert" class="p-3 rounded-lg bg-destructive/10 border border-destructive/20 text-destructive text-sm">
        {error}
      </div>
    {/if}

    <div class="space-y-1.5">
      <label for="sb-name" class="text-sm font-medium text-foreground">Name</label>
      <input
        id="sb-name"
        type="text"
        bind:value={name}
        placeholder="coding-agent"
        class="w-full px-3 py-2 rounded-lg bg-background-1 border border-line-2 focus:border-primary focus:ring-2 focus:ring-primary/20 outline-hidden transition-all text-sm text-foreground"
        disabled={creating}
      />
    </div>

    <div class="space-y-1.5">
      <label for="sb-image" class="text-sm font-medium text-foreground">Image</label>
      <select id="sb-image" bind:value={imageName} disabled={creating || imagesLoading}
        class="w-full px-3 py-2 rounded-lg bg-background-1 border border-line-2 focus:border-primary outline-hidden text-sm text-foreground">
        <option value="">Plain VM</option>
        {#each images as image (image.name)}
          <option value={image.name} disabled={!image.image}>{image.name}{image.image ? '' : ' — incompatible architecture/runtime'}</option>
        {/each}
      </select>
      {#if imagesLoading}
        <p class="text-xs text-muted-foreground-1" role="status">Loading image catalog...</p>
      {:else if imagesError}
        <p class="text-xs text-destructive break-words" role="alert">{imagesError}</p>
      {:else if selectedImage}
        <p class="text-xs text-muted-foreground-1">{selectedImage.description}</p>
        <p class="text-xs text-muted-foreground-1">Compatible · Cache: {selectedImage.cached === ImageCacheState.UNKNOWN ? 'unknown' : selectedImage.cached}</p>
        <p class="text-xs text-muted-foreground break-all">{selectedImage.image}</p>
      {:else}
        <p class="text-xs text-muted-foreground-1">Uses the service's default environment.</p>
      {/if}
    </div>

    <div class="grid grid-cols-2 gap-4">
      <div class="space-y-1.5">
        <label for="sb-ram" class="text-sm font-medium text-foreground">RAM (MB)</label>
        <select
          id="sb-ram"
          bind:value={ramMb}
          class="w-full px-3 py-2 rounded-lg bg-background-1 border border-line-2 focus:border-primary outline-hidden text-sm text-foreground"
          disabled={creating}
        >
          <option value={1024}>1024 MB (1 GB)</option>
          <option value={2048}>2048 MB (2 GB)</option>
          <option value={4096}>4096 MB (4 GB)</option>
          <option value={8192}>8192 MB (8 GB)</option>
          <option value={12288}>12288 MB (12 GB)</option>
          <option value={16384}>16384 MB (16 GB)</option>
        </select>
      </div>

      <div class="space-y-1.5">
        <label for="sb-cpus" class="text-sm font-medium text-foreground">CPUs</label>
        <select
          id="sb-cpus"
          bind:value={cpus}
          class="w-full px-3 py-2 rounded-lg bg-background-1 border border-line-2 focus:border-primary outline-hidden text-sm text-foreground"
          disabled={creating}
        >
          <option value={1}>1 CPU</option>
          <option value={2}>2 CPUs</option>
          <option value={4}>4 CPUs</option>
          <option value={8}>8 CPUs</option>
        </select>
      </div>
    </div>
  </div>
</Modal>
