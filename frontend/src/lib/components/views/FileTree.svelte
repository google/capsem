<script lang="ts">
  import Folder from 'phosphor-svelte/lib/Folder';
  import FolderOpen from 'phosphor-svelte/lib/FolderOpen';
  import FileText from 'phosphor-svelte/lib/File';
  import FileTree from './FileTree.svelte';
  import type { MockFileNode } from '../../mock.ts';

  let { nodes, depth = 0, selectedPath, onSelect }: {
    nodes: MockFileNode[];
    depth?: number;
    selectedPath: string | null;
    onSelect: (node: MockFileNode) => void;
  } = $props();

  let expanded = $state<Record<string, boolean>>({});

  function toggle(node: MockFileNode) {
    if (node.type === 'directory') {
      expanded[node.path] = !expanded[node.path];
    } else {
      onSelect(node);
    }
  }

  function handleKeydown(e: KeyboardEvent, node: MockFileNode) {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      toggle(node);
    }
    if (node.type === 'directory') {
      if (e.key === 'ArrowRight' && !expanded[node.path]) {
        expanded[node.path] = true;
      }
      if (e.key === 'ArrowLeft' && expanded[node.path]) {
        expanded[node.path] = false;
      }
    }
  }
</script>

<ul class="list-none m-0 p-0" role="tree">
  {#each nodes as node}
    <li role="treeitem" aria-selected={selectedPath === node.path} aria-expanded={node.type === 'directory' ? expanded[node.path] ?? false : undefined}>
      <button
        type="button"
        class="w-full flex items-center gap-x-1.5 py-1 px-2 text-sm rounded-lg transition-colors
          {selectedPath === node.path
            ? 'bg-primary/10 text-primary'
            : 'text-foreground hover:bg-muted-hover'}"
        style="padding-left: {depth * 16 + 8}px"
        onclick={() => toggle(node)}
        onkeydown={(e) => handleKeydown(e, node)}
      >
        {#if node.type === 'directory'}
          {#if expanded[node.path]}
            <FolderOpen size={16} class="shrink-0 text-primary" />
          {:else}
            <Folder size={16} class="shrink-0 text-muted-foreground-1" />
          {/if}
        {:else}
          <FileText size={16} class="shrink-0 text-muted-foreground" />
        {/if}
        <span class="truncate">{node.name}</span>
      </button>

      {#if node.type === 'directory' && expanded[node.path] && node.children}
        <FileTree
          nodes={node.children}
          depth={depth + 1}
          {selectedPath}
          {onSelect}
        />
      {/if}
    </li>
  {/each}
</ul>
