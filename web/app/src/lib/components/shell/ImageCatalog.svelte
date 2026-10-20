<script lang="ts">
  import { onMount } from 'svelte';
  import { ImageCacheState, type ImageInfo } from '@capsem/sdk';
  import { getImages, pullImage } from '../../api';
  import { vmStore } from '../../stores/vms.svelte';
  import { sessionActionError } from '../../models/session-action-error';

  let { ready }: { ready: boolean } = $props();
  let images = $state<ImageInfo[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let controller: AbortController | null = null;
  let caching = $state<string | null>(null);
  let cacheError = $state<string | null>(null);
  let cachedImage = $state<string | null>(null);
  let observationTimer: ReturnType<typeof setTimeout> | null = null;

  async function cache(image: ImageInfo) {
    if (!image.image || caching) return;
    caching = image.name;
    cacheError = null;
    cachedImage = null;
    try {
      await pullImage(image.image);
      cachedImage = image.name;
      await load();
    } catch (e) {
      cacheError = sessionActionError(e);
    } finally {
      caching = null;
    }
  }

  async function load(refresh = false, quiet = false) {
    if (observationTimer) clearTimeout(observationTimer);
    controller?.abort();
    const request = new AbortController();
    controller = request;
    if (!quiet) loading = true;
    error = null;
    try {
      const result = await getImages(request.signal, refresh);
      if (!request.signal.aborted) {
        images = result.images;
        // Cache observation runs outside the polled route. Follow its pending
        // result without repeatedly fetching the registry or hiding the cards.
        if (images.some(image => image.image && image.cached === ImageCacheState.UNKNOWN)) {
          observationTimer = setTimeout(() => void load(false, true), 1000);
        }
      }
    } catch (e) {
      if (!request.signal.aborted) error = sessionActionError(e);
    } finally {
      if (!request.signal.aborted) loading = false;
    }
  }

  onMount(() => {
    void load();
    return () => {
      controller?.abort();
      if (observationTimer) clearTimeout(observationTimer);
    };
  });
</script>

<section class="mb-6" aria-labelledby="image-catalog-title" aria-busy={loading}>
  <div class="flex items-center justify-between mb-3 gap-3">
    <h3 id="image-catalog-title" class="text-lg font-medium text-foreground">Images</h3>
    <button type="button" class="text-xs text-primary hover:underline disabled:opacity-50"
      disabled={loading} onclick={() => load(true)}>
      {error ? 'Retry image catalog' : 'Refresh images'}
    </button>
  </div>
  {#if caching}
    <p class="mb-3 text-sm text-muted-foreground-1" role="status">Caching {caching}... The first download can take several minutes.</p>
  {:else if cacheError}
    <p class="mb-3 text-sm text-destructive break-words" role="alert">{cacheError}</p>
  {:else if cachedImage}
    <p class="mb-3 text-sm text-muted-foreground-1" role="status">{cachedImage} is cached and ready.</p>
  {/if}
  {#if loading}
    <p class="text-sm text-muted-foreground-1" role="status">Loading image catalog...</p>
  {:else if error}
    <p class="text-sm text-destructive break-words" role="alert">{error}</p>
  {:else if images.length === 0}
    <p class="text-sm text-muted-foreground-1">No images are available in the catalog. Refresh after images have been published.</p>
  {:else}
    <div class="grid gap-3 sm:grid-cols-2">
      {#each images as image (image.name)}
        <article class="bg-card border border-card-line rounded-xl p-4 flex flex-col gap-2">
          <h4 class="text-sm font-semibold text-foreground">{image.name}</h4>
          <p class="text-xs text-muted-foreground-1 flex-1">{image.description}</p>
          <p class="text-xs text-muted-foreground-1">Architectures: {image.architectures.join(', ')}</p>
          <p class="text-xs text-muted-foreground-1">
            {image.image ? `Cache: ${image.cached === ImageCacheState.UNKNOWN ? 'unknown' : image.cached}` : 'Incompatible with this host architecture or runtime'}
          </p>
          {#if !ready && image.image}
            <p class="text-xs text-muted-foreground-1">VM assets must be ready before creating a session.</p>
          {/if}
          <div class="flex flex-wrap gap-2">
          <button type="button" class="text-xs rounded-lg px-3 py-1.5 bg-primary text-primary-foreground hover:bg-primary-hover disabled:opacity-50"
            aria-label={`New session from ${image.name}`} disabled={!ready || !image.image || vmStore.acting}
            onclick={() => vmStore.openCreateModal(image.name)}>
            New session
          </button>
          <button type="button" class="text-xs rounded-lg px-3 py-1.5 border border-line-2 text-foreground hover:bg-muted-hover disabled:opacity-50"
            aria-label={`Cache ${image.name}`} disabled={!image.image || caching !== null || image.cached === ImageCacheState.READY}
            onclick={() => cache(image)}>
            {caching === image.name ? 'Caching...' : image.cached === ImageCacheState.READY ? 'Cached' : 'Cache image'}
          </button>
          </div>
        </article>
      {/each}
    </div>
  {/if}
</section>
