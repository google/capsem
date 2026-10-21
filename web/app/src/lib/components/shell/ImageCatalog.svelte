<script lang="ts">
  import { onMount } from 'svelte';
  import { ImageCacheState, ImageSurface, type ImageInfo } from '@capsem/sdk';
  import { getImages, pullImage, getContainerStatus, getBaseUrl, openSurface } from '../../api';
  import { vmStore } from '../../stores/vms.svelte';
  import { tabStore } from '../../stores/tabs.svelte';
  import { sessionActionError } from '../../models/session-action-error';
  import { imageNeedsDownload, imageLaunchRequest, imageInterfaceLabel } from '../../models/image-launch';
  import { imageDownloadView } from '../../models/image-download';
  import { surfaceLauncherUrl, waitForSurface } from '../../models/surface';
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
      let appWindow: Window | null = null;
      try {
        const graphical = image.surface === ImageSurface.XPRA;
        if (graphical && !('__TAURI_INTERNALS__' in window)) {
          // Reserve the tab during the click, before provisioning consumes
          // the browser's user activation and its popup allowance.
          appWindow = window.open('about:blank', '_blank');
          if (!appWindow) throw new Error('Allow browser popups to open this graphical app.');
          appWindow.opener = null;
          appWindow.document.title = image.name;
          appWindow.document.body.textContent = `Starting ${image.name}...`;
        }
        const { id, name } = await vmStore.provision(imageLaunchRequest(image));
        tabStore.openVM(id, name);
        if (graphical) {
          await waitForSurface(() => getContainerStatus(id));
          if (appWindow && !appWindow.closed) appWindow.location.replace(surfaceLauncherUrl(getBaseUrl(), id));
          else await openSurface(id);
        }
      } catch (e) {
        appWindow?.close();
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
    void load(false, true);
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
      }
    } catch (e) {
      if (!request.signal.aborted && (!quiet || images.length === 0)) error = sessionActionError(e);
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
        {@const download = imageDownloadView(image.progress)}
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
              aria-label={`${imageNeedsDownload(image, ready) ? 'Download' : 'Launch'} ${image.name}`}
              disabled={caching !== null || launching !== null || vmStore.acting}
              onclick={() => activate(image)}>
              {#if launching === image.name}
                <CircleNotch size={14} class="animate-spin" />
                Launching...
              {:else if imageNeedsDownload(image, ready)}
                <DownloadSimple size={14} /> Download
              {:else}
                <Play size={14} weight="fill" /> Launch
              {/if}
            </button>
            {/if}
          </div>
          {#if failedImage === image.name && cacheError}
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
