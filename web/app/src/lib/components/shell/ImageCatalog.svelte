<script lang="ts">
  import { onMount } from 'svelte';
  import { ImageCacheState, ImageSurface, type ImageInfo, type ImageDownloadProgress } from '@capsem/sdk';
  import { getImages, pullImage } from '../../api';
  import { vmStore } from '../../stores/vms.svelte';
  import { tabStore } from '../../stores/tabs.svelte';
  import { sessionActionError } from '../../models/session-action-error';
  import { imageNeedsDownload, imageLaunchRequest, imageInterfaceLabel, retainImageActionFailure, startImageSession, imageHasUpdate, type ImageActionFailure } from '../../models/image-launch';
  import { imageDownloadView } from '../../models/image-download';
  import { watchVisibleRefresh } from '../../models/visible-refresh';
  import {catalogGroups,imageDisplayName} from '../../models/image-metadata';
  import AppIcon from './AppIcon.svelte';
  import ImageInfoPanel from './ImageInfoPanel.svelte';
  import CircleNotch from 'phosphor-svelte/lib/CircleNotch';
  import DownloadSimple from 'phosphor-svelte/lib/DownloadSimple';
  import Play from 'phosphor-svelte/lib/Play';

  let { ready, prepareRuntime }: { ready: boolean; prepareRuntime: () => Promise<void> } = $props();
  let images = $state<ImageInfo[]>([]);
  let infoImage = $state<ImageInfo|null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let controller: AbortController | null = null;
  let caching = $state<string | null>(null);
  let lastProgress = $state<ImageDownloadProgress | null>(null);
  let failure = $state<ImageActionFailure | null>(null);
  let launching = $state<string | null>(null);
  let availableImages = $derived(images.filter(image => image.image));
  let observationTimer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    failure = retainImageActionFailure(failure, images, ready);
  });

  async function activate(image: ImageInfo) {
    if (!image.image || caching || launching || vmStore.acting) return;
    let action: ImageActionFailure['action'] = 'prepare';
    caching = imageNeedsDownload(image, ready) ? image.name : null;
    lastProgress = null;
    failure = null;
    if (caching) void load(false, true);
    try {
      await startImageSession(image, ready, {
        prepareRuntime,
        pullImage,
        launch: async () => {
          action = 'launch';
          caching = null;
          launching = image.name;
          const { id, name } = await vmStore.provision(imageLaunchRequest(image));
          tabStore.openVM(id, name, image.surface === ImageSurface.XPRA ? 'ui' : 'terminal');
        },
      });
    } catch (e) {
      failure = { image: image.name, action, message: sessionActionError(e) };
    } finally {
      caching = null;
      launching = null;
      lastProgress = null;
      void load(false, true);
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
        const progress = result.images.find(image => image.name === caching)?.progress;
        if (progress) lastProgress = progress;
        images = result.images;
        // Cache observation runs outside the polled route. Follow its pending
        // result without repeatedly fetching the registry or hiding the cards.
      }
    } catch (e) {
      if (!request.signal.aborted) error = sessionActionError(e);
    } finally {
      if (!request.signal.aborted) {
        loading = false;
        const downloading = caching !== null || images.some(image => image.progress);
        if (downloading || images.some(image => image.image && image.cached === ImageCacheState.UNKNOWN)) {
          observationTimer = setTimeout(() => void load(false, true), downloading ? 500 : 1000);
        }
      }
    }
  }

  onMount(() => {
    void load();
    const stopRefreshing = watchVisibleRefresh(async () => {
      if (!loading && !caching && !images.some(image => image.progress || image.cached === ImageCacheState.UNKNOWN)) await load(false, true);
    }, 30_000);
    return () => {
      stopRefreshing();
      controller?.abort();
      if (observationTimer) clearTimeout(observationTimer);
    };
  });
</script>

<section class="mb-6" aria-labelledby="image-catalog-title" aria-busy={loading}>
  <div class="flex items-center justify-between mb-3 gap-3">
    <h3 id="image-catalog-title" class="text-lg font-medium text-foreground">New session</h3>
    {#if error}
      <button type="button" class="text-xs text-primary hover:underline disabled:opacity-50"
        disabled={loading} onclick={() => load(true)}>Retry</button>
    {/if}
  </div>
  {#if loading}
    <p class="text-sm text-muted-foreground-1" role="status">Loading image catalog...</p>
  {:else if error}
    <p class="text-sm text-destructive break-words" role="alert">{error}</p>
  {:else if availableImages.length === 0}
    <p class="text-sm text-muted-foreground-1">No apps are available for this computer.</p>
  {:else}
    <div class="space-y-4">
      {#each catalogGroups(availableImages) as group (group.label)}
      <section aria-label={group.label}>
      <h4 class="mb-2 text-xs font-semibold text-foreground uppercase tracking-wider">{group.label}</h4>
      <div class="grid grid-cols-1 sm:grid-cols-2 gap-3">
      {#each group.images as image (image.name)}
        {@const download = imageDownloadView(image.progress, caching === image.name ? lastProgress : null)}
        <article class="rounded-xl border border-card-line px-3 py-3">
          <div class="flex items-center gap-3">
            <AppIcon icon={image.app?.icon} />
            <div class="min-w-0 flex-1">
              <div class="flex items-center gap-2">
                <h5 class="text-sm font-semibold text-foreground truncate">{imageDisplayName(image)}</h5>
                {#if imageHasUpdate(image)}
                  <span class="shrink-0 rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] text-primary" title="A newer app image is available">Updated</span>
                {/if}
                {#if imageInterfaceLabel(image)}
                  <span class="shrink-0 text-[10px] text-muted-foreground-1">{imageInterfaceLabel(image)}</span>
                {/if}
              </div>
              {#if image.app?.publisher}<p class="text-[11px] text-muted-foreground-1">{image.app.publisher}</p>{/if}
              <p class="text-xs text-muted-foreground-1 truncate" title={image.description}>{image.description}</p>
            </div>
            <button type="button" class="shrink-0 text-xs text-muted-foreground-1 hover:text-primary" aria-label={`Info about ${imageDisplayName(image)}`} onclick={()=>infoImage=image}>Info</button>
            {#if caching === image.name || image.progress}
              <div class="shrink-0 w-24">
                <div class="app-download-progress bg-muted rounded-full" class:indeterminate={download.percent === null}
                  role="progressbar" aria-label={`${download.phase} ${image.name}`}
                  aria-valuemin={download.percent === null ? undefined : 0}
                  aria-valuemax={download.percent === null ? undefined : 100}
                  aria-valuenow={download.percent ?? undefined} aria-valuetext={download.valueText}>
                  {#if download.percent !== null}<span style:width={`${download.percent}%`}></span>{/if}
                </div>
                <span class="block mt-1 text-[10px] text-center text-muted-foreground-1" title={download.valueText}>{download.label}</span>
              </div>
            {:else}
            <button type="button" class="shrink-0 inline-flex items-center justify-center gap-1.5 min-w-24 text-xs rounded-lg px-3 py-2 bg-primary text-primary-foreground hover:bg-primary-hover disabled:opacity-50"
              aria-label={`New session with ${imageDisplayName(image)}`}
              disabled={caching !== null || launching !== null || vmStore.acting}
              onclick={() => activate(image)}>
              {#if launching === image.name}
                <CircleNotch size={14} class="animate-spin" />
                New session
              {:else if imageNeedsDownload(image, ready)}
                <DownloadSimple size={14} /> New session
              {:else}
                <Play size={14} weight="fill" /> New session
              {/if}
            </button>
            {/if}
          </div>
          {#if failure?.image === image.name}
            <div class="mt-2 text-xs text-destructive break-words" role="alert">
              <p>Couldn't {failure.action === 'launch' ? 'launch' : 'prepare'} this app. Try again.</p>
              <details class="mt-1"><summary class="cursor-pointer">Details</summary>{failure.message}</details>
            </div>
          {/if}
        </article>
      {/each}
      </div></section>
      {/each}
    </div>
  {/if}
</section>
{#if infoImage}<ImageInfoPanel image={infoImage} onclose={()=>infoImage=null} />{/if}

<style>
  .app-download-progress {
    position: relative;
    overflow: hidden;
    width: 100%;
    height: 6px;
  }
  .app-download-progress > span {
    display: block;
    height: 100%;
    background: var(--primary);
    border-radius: inherit;
    transition: width 150ms linear;
  }
  .app-download-progress.indeterminate::after {
    content: '';
    position: absolute;
    width: 40%;
    height: 100%;
    border-radius: inherit;
    background: var(--primary);
    animation: app-download 1.2s ease-in-out infinite alternate;
  }
  @keyframes app-download {
    from { transform: translateX(0); }
    to { transform: translateX(150%); }
  }
  @media (prefers-reduced-motion: reduce) {
    .app-download-progress.indeterminate::after { animation: none; left: 30%; }
    .app-download-progress > span { transition: none; }
  }
</style>
