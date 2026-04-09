<script lang="ts">
  import FileTree from './FileTree.svelte';
  import FileContent from './FileContent.svelte';
  import { mockFileTree, findFileNode } from '../../mock.ts';
  import type { MockFileNode } from '../../mock.ts';

  let { vmId }: { vmId: string } = $props();

  let selectedPath = $state<string | null>(null);
  let selectedNode = $derived(selectedPath ? findFileNode(mockFileTree, selectedPath) ?? null : null);

  function handleSelect(node: MockFileNode) {
    selectedPath = node.path;
  }
</script>

<div class="flex h-full">
  <!-- Tree pane -->
  <div class="w-64 shrink-0 border-r border-line-2 overflow-auto bg-layer">
    <div class="px-3 py-2 border-b border-line-2">
      <span class="text-xs font-medium text-muted-foreground uppercase tracking-wider">Files</span>
    </div>
    <div class="py-1">
      <FileTree nodes={mockFileTree} {selectedPath} onSelect={handleSelect} />
    </div>
  </div>

  <!-- Content pane -->
  <div class="flex-1 min-w-0">
    <FileContent node={selectedNode} />
  </div>
</div>
