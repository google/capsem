<script lang="ts">
  import { onMount } from 'svelte';
  import { ImageCacheState, type ImageInfo } from '@capsem/sdk';
  import { getImages, pullImage } from '../../api';
  import { vmStore } from '../../stores/vms.svelte';
  import { tabStore } from '../../stores/tabs.svelte';
  import { sessionActionError } from '../../models/session-action-error';
  import { imageNeedsDownload, imageLaunchRequest, imageInterfaceLabel } from '../../models/image-launch';
  import CircleNotch from 'phosphor-svelte/lib/CircleNotch';
  import DownloadSimple from 'phosphor-svelte/lib/DownloadSimple';
  import Play from 'phosphor-svelte/lib/Play';

  let { ready, prepareRuntime }: { ready: boolean; prepareRuntime: () => Promise<void> } = $props();
  let images = $state<ImageInfo[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let controller: AbortController | null = null;
  let caching = $state<string | null>(null);
  let cacheError = $state<string | null>(null);
  let launching = $state<string | null>(null);
  let failedImage = $state<string | null>(null);
  let availableImages = $derived(images.filter(image => image.image));
  let observationTimer: ReturnType<typeof setTimeout> | null = null;

  async function activate(image: ImageInfo) {
    if (!image.image || caching || launching || vmStore.acting) return;
    if (!imageNeedsDownload(image, ready)) {
      launching = image.name;
      cacheError = null;
      failedImage = null;
      try {
        const { id, name } = await vmStore.provision(imageLaunchRequest(image));
        tabStore.openVM(id, name);
      } catch (e) {
        failedImage = image.name;
        cacheError = sessionActionError(e);
      } finally {
        launching = null;
      }
      return;
    }
    caching = image.name;
    cacheError = null;
    failedImage = null;
    try {
      if (!ready) await prepareRuntime();
      if (image.cached !== ImageCacheState.READY) await pullImage(image.image);
      await load(false, true);
    } catch (e) {
      failedImage = image.name;
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
    <h3 id="image-catalog-title" class="text-lg font-medium text-foreground">Apps</h3>
    <button type="button" class="text-xs text-primary hover:underline disabled:opacity-50"
      disabled={loading} onclick={() => load(true)}>
      {error ? 'Retry' : 'Refresh'}
    </button>
  </div>
  {#if loading}
    <p class="text-sm text-muted-foreground-1" role="status">Loading image catalog...</p>
  {:else if error}
    <p class="text-sm text-destructive break-words" role="alert">{error}</p>
  {:else if availableImages.length === 0}
    <p class="text-sm text-muted-foreground-1">No apps are available for this computer. Try refreshing.</p>
  {:else}
    <div class="border border-card-line rounded-xl divide-y divide-card-line overflow-hidden">
      {#each availableImages as image (image.name)}
        <article class="px-4 py-3">
          <div class="flex items-center gap-3">
            <div class="min-w-0 flex-1">
              <div class="flex items-center gap-2">
                <h4 class="text-sm font-semibold text-foreground truncate">{image.name}</h4>
                {#if imageInterfaceLabel(image)}
                  <span class="shrink-0 text-[10px] text-muted-foreground-1">{imageInterfaceLabel(image)}</span>
                {/if}
              </div>
              <p class="text-xs text-muted-foreground-1 truncate" title={image.description}>{image.description}</p>
            </div>
            <button type="button" class="shrink-0 inline-flex items-center justify-center gap-1.5 min-w-24 text-xs rounded-lg px-3 py-2 bg-primary text-primary-foreground hover:bg-primary-hover disabled:opacity-50"
              aria-label={`${imageNeedsDownload(image, ready) ? 'Download' : 'Launch'} ${image.name}`}
              disabled={caching !== null || launching !== null || vmStore.acting}
              onclick={() => activate(image)}>
              {#if caching === image.name || launching === image.name}
                <CircleNotch size={14} class="animate-spin" />
                {caching === image.name ? 'Downloading...' : 'Launching...'}
              {:else if imageNeedsDownload(image, ready)}
                <DownloadSimple size={14} /> Download
              {:else}
                <Play size={14} weight="fill" /> Launch
              {/if}
            </button>
          </div>
          {#if caching === image.name}
            <p class="mt-2 text-xs text-muted-foreground-1" role="status">Downloading. You can leave this page open; the first download may take a few minutes.</p>
          {:else if failedImage === image.name && cacheError}
            <div class="mt-2 text-xs text-destructive break-words" role="alert">
              <p>Couldn't prepare this app. Try again.</p>
              <details class="mt-1"><summary class="cursor-pointer">Details</summary>{cacheError}</details>
            </div>
          {/if}
        </article>
      {/each}
    </div>
  {/if}
</section>
