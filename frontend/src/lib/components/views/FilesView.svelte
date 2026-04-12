<script lang="ts">
  import { onMount } from 'svelte';
  import FileTree from './FileTree.svelte';
  import FileContent from './FileContent.svelte';
  import * as api from '../../api';
  import { mockFileTree, findFileNode } from '../../mock.ts';
  import type { MockFileNode } from '../../mock.ts';

  let { vmId }: { vmId: string } = $props();

  let fileTree = $state<MockFileNode[]>(mockFileTree);
  let selectedPath = $state<string | null>(null);
  let selectedNode = $state<MockFileNode | null>(null);
  let loading = $state(false);

  // When selectedPath changes, fetch file content from API if connected
  async function handleSelect(node: MockFileNode) {
    selectedPath = node.path;
    if (node.type === 'file' && api.isConnected()) {
      try {
        const resp = await api.readFile(vmId, node.path);
        selectedNode = { ...node, content: resp.content };
      } catch {
        selectedNode = findFileNode(fileTree, node.path) ?? null;
      }
    } else {
      selectedNode = findFileNode(fileTree, node.path) ?? null;
    }
  }

  onMount(async () => {
    if (!api.isConnected()) return;
    loading = true;
    try {
      // Build file tree from workspace listing
      const result = await api.execCommand(vmId, 'find /workspace -maxdepth 4 -not -path "*/\\.*" | sort', 10);
      if (result.exit_code === 0 && result.stdout.trim()) {
        const built = buildTreeFromPaths(result.stdout.trim().split('\n'));
        if (built.length > 0) fileTree = built;
      }
    } catch {
      // Keep mock file tree
    } finally {
      loading = false;
    }
  });

  function buildTreeFromPaths(paths: string[]): MockFileNode[] {
    const root: MockFileNode[] = [];
    const nodeMap = new Map<string, MockFileNode>();

    for (const p of paths) {
      if (p === '/workspace') continue;
      const isDir = !p.includes('.') || paths.some(other => other.startsWith(p + '/'));
      const name = p.split('/').pop() ?? p;
      const node: MockFileNode = { name, type: isDir ? 'directory' : 'file', path: p };
      if (isDir) node.children = [];
      nodeMap.set(p, node);

      const parentPath = p.substring(0, p.lastIndexOf('/'));
      const parent = nodeMap.get(parentPath);
      if (parent?.children) {
        parent.children.push(node);
      } else if (parentPath === '/workspace' || parentPath === '') {
        root.push(node);
      }
    }
    return root;
  }
</script>

<div class="flex h-full">
  <!-- Tree pane -->
  <div class="w-64 shrink-0 border-r border-line-2 overflow-auto bg-layer">
    <div class="px-3 py-2 border-b border-line-2">
      <span class="text-xs font-medium text-muted-foreground uppercase tracking-wider">Files</span>
      {#if loading}
        <span class="text-xs text-muted-foreground ml-2">Loading...</span>
      {/if}
    </div>
    <div class="py-1">
      <FileTree nodes={fileTree} {selectedPath} onSelect={handleSelect} />
    </div>
  </div>

  <!-- Content pane -->
  <div class="flex-1 min-w-0">
    <FileContent node={selectedNode} />
  </div>
</div>
