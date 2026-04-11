<script lang="ts">
  import { onMount } from 'svelte';
  import type { Highlighter } from 'shiki';
  import type { MockFileNode } from '../../mock.ts';
  import { themeStore } from '../../stores/theme.svelte.ts';
  import { getShikiHighlighter, resolveShikiTheme, detectShikiLang } from '../../shiki.ts';

  let { node }: { node: MockFileNode | null } = $props();

  let highlighter: Highlighter | null = $state(null);
  let highlightedHtml = $state('');

  onMount(async () => {
    highlighter = await getShikiHighlighter();
  });

  $effect(() => {
    if (!node?.content || !highlighter) {
      highlightedHtml = '';
      return;
    }
    highlightedHtml = highlighter.codeToHtml(node.content, {
      lang: detectShikiLang(node.name),
      theme: resolveShikiTheme(themeStore.terminalTheme, themeStore.mode),
    });
  });

  let breadcrumbs = $derived.by(() => {
    if (!node) return [];
    const parts = node.path.split('/').filter(Boolean);
    return parts.map((part, i) => ({
      label: part,
      path: '/' + parts.slice(0, i + 1).join('/'),
    }));
  });

  function formatSize(bytes: number | undefined): string {
    if (bytes == null) return '';
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
</script>

<div class="flex flex-col h-full">
  {#if node}
    <div class="flex items-center gap-x-1 px-4 py-2 border-b border-line-2 bg-layer text-sm">
      {#each breadcrumbs as crumb, i}
        {#if i > 0}
          <span class="text-muted-foreground">/</span>
        {/if}
        <span class="{i === breadcrumbs.length - 1 ? 'text-foreground font-medium' : 'text-muted-foreground-1'}">{crumb.label}</span>
      {/each}
      {#if node.sizeBytes != null}
        <span class="ml-auto text-xs text-muted-foreground">{formatSize(node.sizeBytes)}</span>
      {/if}
    </div>

    <div class="flex-1 overflow-auto shiki-wrapper">
      {#if node.content && highlightedHtml}
        {@html highlightedHtml}
      {:else if node.content}
        <pre class="px-4 py-2 font-mono text-sm text-foreground whitespace-pre">{node.content}</pre>
      {:else if node.type === 'directory'}
        <div class="flex items-center justify-center h-full">
          <p class="text-muted-foreground">Select a file to view its contents</p>
        </div>
      {:else}
        <div class="flex items-center justify-center h-full">
          <p class="text-muted-foreground">No content available</p>
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
