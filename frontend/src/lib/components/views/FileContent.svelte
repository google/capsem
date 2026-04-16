<script lang="ts">
  import { onMount } from 'svelte';
  import type { Highlighter } from 'shiki';
  import type { FileEntry } from '../../types';
  import { themeStore } from '../../stores/theme.svelte.ts';
  import { getShikiHighlighter, resolveShikiTheme, detectShikiLang } from '../../shiki.ts';
  import { formatBytes } from '../../format';

  let { entry, content, blob }: {
    entry: FileEntry | null;
    content: string | null;
    blob: Blob | null;
  } = $props();

  let highlighter: Highlighter | null = $state(null);
  let highlightedHtml = $state('');

  onMount(async () => {
    highlighter = await getShikiHighlighter();
  });

  $effect(() => {
    if (!content || !highlighter || !entry) {
      highlightedHtml = '';
      return;
    }
    // Use Magika label for better language detection when available
    const langHint = entry.label ?? entry.name;
    highlightedHtml = highlighter.codeToHtml(content, {
      lang: detectShikiLang(langHint),
      theme: resolveShikiTheme(themeStore.terminalTheme, themeStore.mode),
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

  let isText = $derived(entry?.is_text !== false);
  let isBinary = $derived(entry != null && entry.type === 'file' && entry.is_text === false);
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
      {#if entry.size > 0}
        <span class="ml-auto text-xs text-muted-foreground">{formatBytes(entry.size)}</span>
      {/if}
    </div>

    <div class="flex-1 overflow-auto shiki-wrapper">
      {#if isBinary}
        <div class="flex items-center justify-center h-full">
          <p class="text-muted-foreground">Binary file ({formatBytes(entry.size)})</p>
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
