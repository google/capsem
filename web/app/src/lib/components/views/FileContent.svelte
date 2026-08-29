<script lang="ts">
  import { onDestroy } from 'svelte';
  import type { FileEntry } from '../../types';
  import { themeStore } from '../../stores/theme.svelte.ts';
  import { highlightCode, resolveShikiTheme, detectShikiLang } from '../../shiki.ts';
  import { formatBytes } from '../../format';
  import Copy from 'phosphor-svelte/lib/Copy';
  import DownloadSimple from 'phosphor-svelte/lib/DownloadSimple';

  let { entry, content, blob }: {
    entry: FileEntry | null;
    content: string | null;
    blob: Blob | null;
  } = $props();

  let highlightedHtml = $state('');
  let copied = $state(false);
  let blobUrl = $state<string | null>(null);

  // Track previous blob URL for cleanup without reading $state in the effect
  let prevBlobUrl: string | null = null;

  onDestroy(() => {
    if (prevBlobUrl) URL.revokeObjectURL(prevBlobUrl);
  });

  let isImage = $derived(
    entry != null && entry.type === 'file' &&
    entry.mime != null && entry.mime.startsWith('image/') &&
    entry.is_text !== true && entry.label !== 'svg'
  );

  let isSvg = $derived(
    entry != null && entry.type === 'file' &&
    (entry.label === 'svg' || entry.mime === 'image/svg+xml')
  );

  let isBinary = $derived(
    entry != null && entry.type === 'file' &&
    entry.is_text === false && !isImage && !isSvg
  );

  // Blob URL for image/SVG preview
  $effect(() => {
    // Read deps
    const currentBlob = blob;
    const needsUrl = isImage || isSvg;

    // Cleanup previous
    if (prevBlobUrl) {
      URL.revokeObjectURL(prevBlobUrl);
      prevBlobUrl = null;
    }

    if (currentBlob && needsUrl) {
      const url = URL.createObjectURL(currentBlob);
      prevBlobUrl = url;
      blobUrl = url;
    } else {
      blobUrl = null;
    }
  });

  // Syntax highlighting. highlightCode() lazily fetches the grammar +
  // theme on first use. A generation counter lets a later file-change
  // abort the in-flight load without flashing stale highlighted HTML.
  let highlightGen = 0;
  $effect(() => {
    if (!content || !entry || isImage || isSvg) {
      highlightedHtml = '';
      return;
    }
    const gen = ++highlightGen;
    const langHint = entry.label ?? entry.name;
    const lang = detectShikiLang(langHint, content);
    const theme = resolveShikiTheme(themeStore.terminalTheme, themeStore.mode);
    highlightCode(content, lang, theme).then(html => {
      if (gen !== highlightGen) return;
      highlightedHtml = html;
    }).catch(e => {
      if (gen !== highlightGen) return;
      console.error('[FileContent] Shiki highlight failed:', e);
      highlightedHtml = '';
    });
  });

  let breadcrumbs = $derived.by(() => {
    if (!entry) return [];
    const parts = entry.path.split('/').filter(Boolean);
    return parts.map((part, i) => ({
      label: part,
      path: parts.slice(0, i + 1).join('/'),
    }));
  });

  async function copyToClipboard() {
    if (!content) return;
    try {
      await navigator.clipboard.writeText(content);
      copied = true;
      setTimeout(() => { copied = false; }, 1500);
    } catch {
      // Clipboard API not available
    }
  }

  function downloadFile() {
    if (!blob || !entry) return;
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = entry.name;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }
</script>

<div class="flex flex-col h-full">
  {#if entry}
    <div class="flex items-center gap-x-1 px-4 py-2 border-b border-line-2 bg-layer text-sm">
      {#each breadcrumbs as crumb, i}
        {#if i > 0}
          <span class="text-muted-foreground">/</span>
        {/if}
        <span class="{i === breadcrumbs.length - 1 ? 'text-foreground font-medium' : 'text-muted-foreground-1'}">{crumb.label}</span>
      {/each}

      <div class="ml-auto flex items-center gap-x-1">
        {#if entry.size > 0}
          <span class="text-xs text-muted-foreground mr-1">{formatBytes(entry.size)}</span>
        {/if}
        {#if entry.type === 'file'}
          {#if !isBinary && !isImage && content}
            <button
              type="button"
              class="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted-hover transition-colors"
              onclick={copyToClipboard}
              title="Copy to clipboard"
            >
              {#if copied}
                <span class="text-xs text-primary">Copied!</span>
              {:else}
                <Copy size={14} />
              {/if}
            </button>
          {/if}
          {#if blob}
            <button
              type="button"
              class="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted-hover transition-colors"
              onclick={downloadFile}
              title="Download"
            >
              <DownloadSimple size={14} />
            </button>
          {/if}
        {/if}
      </div>
    </div>

    <div class="flex-1 overflow-auto shiki-wrapper">
      {#if (isImage || isSvg) && blobUrl}
        <div class="flex items-center justify-center h-full p-6 bg-background">
          <img
            src={blobUrl}
            alt={entry.name}
            class="max-w-full max-h-full object-contain rounded-md"
          />
        </div>
      {:else if isBinary}
        <div class="flex flex-col items-center justify-center h-full gap-y-3">
          <p class="text-muted-foreground">Binary file ({formatBytes(entry.size)})</p>
          {#if blob}
            <button
              type="button"
              class="inline-flex items-center gap-x-2 px-4 py-2 bg-primary text-primary-foreground rounded-lg text-sm font-medium hover:bg-primary-hover transition-colors"
              onclick={downloadFile}
            >
              <DownloadSimple size={16} />
              Download
            </button>
          {/if}
        </div>
      {:else if content && highlightedHtml}
        {@html highlightedHtml}
      {:else if content}
        <pre class="px-4 py-2 font-mono text-sm text-foreground whitespace-pre">{content}</pre>
      {:else if entry.type === 'directory'}
        <div class="flex items-center justify-center h-full">
          <p class="text-muted-foreground">Select a file to view its contents</p>
        </div>
      {:else}
        <div class="flex items-center justify-center h-full">
          <p class="text-muted-foreground">Loading...</p>
        </div>
      {/if}
    </div>
  {:else}
    <div class="flex items-center justify-center h-full">
      <p class="text-muted-foreground">Select a file to view its contents</p>
    </div>
  {/if}
</div>

<style>
  .shiki-wrapper :global(pre.shiki) {
    margin: 0;
    padding: 0.75rem 1rem;
    background: transparent !important;
    font-size: 0.875rem;
    line-height: 1.5;
  }

  .shiki-wrapper :global(pre.shiki code) {
    counter-reset: line;
  }

  .shiki-wrapper :global(pre.shiki code .line) {
    display: inline-block;
    width: 100%;
    padding: 0 0.5rem;
  }

  .shiki-wrapper :global(pre.shiki code .line::before) {
    counter-increment: line;
    content: counter(line);
    display: inline-block;
    width: 2.5rem;
    margin-right: 1rem;
    text-align: right;
    color: var(--muted-foreground, #6b7280);
    font-size: 0.75rem;
    user-select: none;
  }

  .shiki-wrapper :global(pre.shiki code .line:hover) {
    background: var(--muted-hover, rgba(128, 128, 128, 0.1));
  }
</style>
