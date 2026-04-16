<script lang="ts">
  import Folder from 'phosphor-svelte/lib/Folder';
  import FolderOpen from 'phosphor-svelte/lib/FolderOpen';
  import FileText from 'phosphor-svelte/lib/File';
  import FileTree from './FileTree.svelte';
  import type { FileEntry } from '../../types';
  import { formatBytes } from '../../format';

  let { entries, depth = 0, selectedPath, onSelect }: {
    entries: FileEntry[];
    depth?: number;
    selectedPath: string | null;
    onSelect: (entry: FileEntry) => void;
  } = $props();

  let expanded = $state<Record<string, boolean>>({});

  function toggle(entry: FileEntry) {
    if (entry.type === 'directory') {
      expanded[entry.path] = !expanded[entry.path];
    } else {
      onSelect(entry);
    }
  }

  function handleKeydown(e: KeyboardEvent, entry: FileEntry) {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      toggle(entry);
    }
    if (entry.type === 'directory') {
      if (e.key === 'ArrowRight' && !expanded[entry.path]) {
        expanded[entry.path] = true;
      }
      if (e.key === 'ArrowLeft' && expanded[entry.path]) {
        expanded[entry.path] = false;
      }
    }
  }
</script>

<ul class="list-none m-0 p-0" role="tree">
  {#each entries as entry}
    <li role="treeitem" aria-selected={selectedPath === entry.path} aria-expanded={entry.type === 'directory' ? expanded[entry.path] ?? false : undefined}>
      <button
        type="button"
        class="w-full flex items-center gap-x-1.5 py-1 px-2 text-sm rounded-lg transition-colors
          {selectedPath === entry.path
            ? 'bg-primary/10 text-primary'
            : 'text-foreground hover:bg-muted-hover'}"
        style="padding-left: {depth * 16 + 8}px"
        onclick={() => toggle(entry)}
        onkeydown={(e) => handleKeydown(e, entry)}
      >
        {#if entry.type === 'directory'}
          {#if expanded[entry.path]}
            <FolderOpen size={16} class="shrink-0 text-primary" />
          {:else}
            <Folder size={16} class="shrink-0 text-muted-foreground-1" />
          {/if}
        {:else}
          <FileText size={16} class="shrink-0 text-muted-foreground" />
        {/if}
        <span class="truncate">{entry.name}</span>
        {#if entry.type === 'file' && entry.size > 0}
          <span class="ml-auto text-xs text-muted-foreground shrink-0">{formatBytes(entry.size)}</span>
        {/if}
      </button>

      {#if entry.type === 'directory' && expanded[entry.path] && entry.children}
        <FileTree
          entries={entry.children}
          depth={depth + 1}
          {selectedPath}
          {onSelect}
        />
      {/if}
    </li>
  {/each}
</ul>
