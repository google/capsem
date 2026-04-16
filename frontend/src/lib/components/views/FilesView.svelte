<script lang="ts">
  import { onMount } from 'svelte';
  import FileTree from './FileTree.svelte';
  import FileContent from './FileContent.svelte';
  import ArrowClockwise from 'phosphor-svelte/lib/ArrowClockwise';
  import UploadSimple from 'phosphor-svelte/lib/UploadSimple';
  import * as api from '../../api';
  import type { FileEntry } from '../../types';

  let { vmId }: { vmId: string } = $props();

  let fileTree = $state<FileEntry[]>([]);
  let selectedPath = $state<string | null>(null);
  let selectedEntry = $state<FileEntry | null>(null);
  let fileContent = $state<string | null>(null);
  let fileBlob = $state<Blob | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let dragActive = $state(false);
  let uploadStatus = $state<string | null>(null);
  let dragCounter = $state(0);

  function findEntry(tree: FileEntry[], path: string): FileEntry | undefined {
    for (const node of tree) {
      if (node.path === path) return node;
      if (node.children) {
        const found = findEntry(node.children, path);
        if (found) return found;
      }
    }
    return undefined;
  }

  async function loadTree() {
    if (!api.isConnected()) return;
    loading = true;
    error = null;
    try {
      const resp = await api.listFiles(vmId, '/', 4);
      fileTree = resp.entries;
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to load files';
      fileTree = [];
    } finally {
      loading = false;
    }
  }

  async function handleSelect(entry: FileEntry) {
    selectedPath = entry.path;
    selectedEntry = entry;
    fileContent = null;
    fileBlob = null;

    if (entry.type === 'file' && api.isConnected()) {
      try {
        const result = await api.getFileContent(vmId, entry.path);
        fileContent = result.text;
        fileBlob = result.blob;
      } catch {
        fileContent = null;
        fileBlob = null;
      }
    }
  }

  async function refresh() {
    await loadTree();
    if (selectedPath) {
      const found = findEntry(fileTree, selectedPath);
      if (!found) {
        selectedPath = null;
        selectedEntry = null;
        fileContent = null;
        fileBlob = null;
      }
    }
  }

  // Determine upload target directory from selection
  function getUploadDir(): string {
    if (selectedEntry?.type === 'directory') return selectedEntry.path;
    if (selectedEntry?.path) {
      const lastSlash = selectedEntry.path.lastIndexOf('/');
      return lastSlash > 0 ? selectedEntry.path.substring(0, lastSlash) : '';
    }
    return '';
  }

  function handleDragEnter(e: DragEvent) {
    e.preventDefault();
    dragCounter += 1;
    if (e.dataTransfer?.types.includes('Files')) {
      dragActive = true;
    }
  }

  function handleDragOver(e: DragEvent) {
    e.preventDefault();
    if (e.dataTransfer) {
      e.dataTransfer.dropEffect = 'copy';
    }
  }

  function handleDragLeave(e: DragEvent) {
    e.preventDefault();
    dragCounter -= 1;
    if (dragCounter <= 0) {
      dragActive = false;
      dragCounter = 0;
    }
  }

  async function handleDrop(e: DragEvent) {
    e.preventDefault();
    dragActive = false;
    dragCounter = 0;

    const files = e.dataTransfer?.files;
    if (!files || files.length === 0) return;

    const dir = getUploadDir();
    let uploaded = 0;
    let failed = 0;

    uploadStatus = `Uploading ${files.length} file${files.length > 1 ? 's' : ''}...`;

    for (const file of files) {
      const sanitized = api.sanitizePath(file.name);
      if (!sanitized) {
        failed++;
        continue;
      }
      const targetPath = dir ? `${dir}/${sanitized}` : sanitized;
      try {
        await api.uploadFile(vmId, targetPath, file);
        uploaded++;
      } catch {
        failed++;
      }
    }

    if (failed > 0) {
      uploadStatus = `Uploaded ${uploaded}, failed ${failed}`;
    } else {
      uploadStatus = `Uploaded ${uploaded} file${uploaded > 1 ? 's' : ''}`;
    }
    setTimeout(() => { uploadStatus = null; }, 3000);

    await refresh();
  }

  onMount(() => {
    loadTree();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="flex h-full relative"
  ondragenter={handleDragEnter}
  ondragover={handleDragOver}
  ondragleave={handleDragLeave}
  ondrop={handleDrop}
>
  {#if dragActive}
    <div class="absolute inset-0 z-50 flex items-center justify-center bg-background/80 border-2 border-dashed border-primary rounded-lg pointer-events-none">
      <div class="flex flex-col items-center gap-y-2">
        <UploadSimple size={32} class="text-primary" />
        <p class="text-sm font-medium text-primary">Drop files to upload</p>
      </div>
    </div>
  {/if}

  <!-- Tree pane -->
  <div class="w-64 shrink-0 border-r border-line-2 overflow-auto bg-layer">
    <div class="flex items-center justify-between px-3 py-2 border-b border-line-2">
      <span class="text-xs font-medium text-muted-foreground uppercase tracking-wider">Files</span>
      <div class="flex items-center gap-x-1">
        {#if uploadStatus}
          <span class="text-xs text-primary">{uploadStatus}</span>
        {:else if loading}
          <span class="text-xs text-muted-foreground">Loading...</span>
        {/if}
        <button
          type="button"
          class="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted-hover transition-colors"
          onclick={refresh}
          title="Refresh"
        >
          <ArrowClockwise size={14} class={loading ? 'animate-spin' : ''} />
        </button>
      </div>
    </div>
    {#if error}
      <div class="px-3 py-4 text-sm text-destructive">{error}</div>
    {:else if !loading && fileTree.length === 0}
      <div class="px-3 py-4 text-sm text-muted-foreground">No files in workspace</div>
    {:else}
      <div class="py-1">
        <FileTree entries={fileTree} {selectedPath} onSelect={handleSelect} />
      </div>
    {/if}
  </div>

  <!-- Content pane -->
  <div class="flex-1 min-w-0">
    <FileContent
      entry={selectedEntry}
      content={fileContent}
      blob={fileBlob}
    />
  </div>
</div>
