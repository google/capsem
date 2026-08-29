<script lang="ts">
  import type { SettingsLeaf, SettingValue } from '../../../types/settings';
  import { themeStore } from '../../../stores/theme.svelte.ts';
  import { highlightCode, resolveShikiTheme, detectShikiLang } from '../../../shiki.ts';
  import Copy from 'phosphor-svelte/lib/Copy';
  import Check from 'phosphor-svelte/lib/Check';
  import PencilSimple from 'phosphor-svelte/lib/PencilSimple';
  import X from 'phosphor-svelte/lib/X';
  import FloppyDisk from 'phosphor-svelte/lib/FloppyDisk';
  import FileText from 'phosphor-svelte/lib/FileText';

  let { leaf, disabled = false, onchange }: {
    leaf: SettingsLeaf;
    disabled?: boolean;
    onchange: (value: SettingValue) => void;
  } = $props();

  let highlightedHtml = $state('');
  let isEditing = $state(false);
  let draftContent = $state('');
  let draftPath = $state('');
  let copied = $state(false);

  function fileValue(): { path: string; content: string } {
    const v = leaf.effective_value;
    if (typeof v === 'object' && v !== null && 'path' in v) return v as { path: string; content: string };
    return { path: '', content: String(v) };
  }

  let fv = $derived(fileValue());
  let filetype = $derived(leaf.metadata.filetype ?? detectFiletype());

  function detectFiletype(): string {
    const ext = fv.path.split('.').pop()?.toLowerCase() ?? '';
    if (ext === 'json') return 'json';
    if (ext === 'sh' || ext === 'bashrc' || fv.path.endsWith('.bashrc')) return 'bash';
    if (ext === 'conf') return 'conf';
    if (ext === 'toml') return 'toml';
    return 'text';
  }

  function formatJson(text: string): string {
    try { return JSON.stringify(JSON.parse(text), null, 2); } catch { return text; }
  }

  function compactJson(text: string): string {
    try { return JSON.stringify(JSON.parse(text)); } catch { return text.trim(); }
  }

  let highlightGen = 0;
  $effect(() => {
    const content = fv.content;
    const editing = isEditing;
    const ft = filetype;
    const termTheme = themeStore.terminalTheme;
    const mode = themeStore.mode;

    if (!content || editing) {
      highlightedHtml = '';
      return;
    }
    const gen = ++highlightGen;
    const formatted = ft === 'json' ? formatJson(content) : content;
    const lang = detectShikiLang(ft);
    const theme = resolveShikiTheme(termTheme, mode);
    highlightCode(formatted, lang, theme).then(html => {
      if (gen !== highlightGen) return;
      highlightedHtml = html;
    }).catch(e => {
      if (gen !== highlightGen) return;
      console.error('[FileEditorControl] Shiki highlight failed:', e);
      highlightedHtml = '';
    });
  });

  function startEditing() {
    draftContent = filetype === 'json' ? formatJson(fv.content) : fv.content;
    draftPath = fv.path;
    isEditing = true;
  }

  function cancelEditing() {
    isEditing = false;
    draftContent = '';
    draftPath = '';
  }

  function saveFile() {
    const content = filetype === 'json' ? compactJson(draftContent) : draftContent.trim();
    onchange({ path: draftPath, content });
    isEditing = false;
  }

  async function copyContent() {
    const text = isEditing ? draftContent : (filetype === 'json' ? formatJson(fv.content) : fv.content);
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
      setTimeout(() => { copied = false; }, 1500);
    } catch { /* clipboard may not be available */ }
  }

  let lineCount = $derived.by(() => {
    const content = filetype === 'json' ? formatJson(fv.content) : fv.content;
    return Math.min(Math.max(content.split('\n').length + 1, 4), 20);
  });
</script>

<div class="py-2">
  <div class="bg-card border border-card-line rounded-xl overflow-hidden">
    <!-- Header -->
    <div class="flex items-center gap-x-2 px-3 py-2 border-b border-card-divider bg-background-1">
      <FileText size={14} class="text-muted-foreground-1 shrink-0" />
      <span class="text-xs font-medium text-foreground flex-1 min-w-0 truncate">{leaf.name}</span>
      {#if isEditing}
        <input
          type="text"
          class="text-[10px] font-mono text-muted-foreground-1 bg-transparent border-b border-line-2 focus:outline-hidden focus:border-primary w-48 text-right px-0.5"
          value={draftPath}
          oninput={(e) => { draftPath = (e.target as HTMLInputElement).value; }}
        />
      {:else if fv.path}
        <span class="text-[10px] font-mono text-muted-foreground-1 truncate max-w-48">{fv.path}</span>
      {/if}
      {#if leaf.corp_locked}
        <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-destructive/10 text-destructive font-medium">corp</span>
      {/if}
      <!-- Copy -->
      <button
        type="button"
        class="p-1 rounded-md text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover transition-colors"
        onclick={copyContent}
        title="Copy to clipboard"
      >
        {#if copied}
          <Check size={14} class="text-primary" />
        {:else}
          <Copy size={14} />
        {/if}
      </button>
      <!-- Edit / Save / Cancel -->
      {#if !disabled}
        {#if isEditing}
          <button
            type="button"
            class="py-0.5 px-2 text-xs rounded-md text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover transition-colors"
            onclick={cancelEditing}
          >
            <X size={14} />
          </button>
          <button
            type="button"
            class="py-0.5 px-2 text-xs rounded-md bg-primary text-primary-foreground hover:bg-primary-hover transition-colors"
            onclick={saveFile}
          >
            <FloppyDisk size={14} />
          </button>
        {:else}
          <button
            type="button"
            class="p-1 rounded-md text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover transition-colors"
            onclick={startEditing}
            title="Edit"
          >
            <PencilSimple size={14} />
          </button>
        {/if}
      {/if}
    </div>
    <!-- Content -->
    {#if isEditing}
      <textarea
        class="w-full bg-transparent font-mono text-xs leading-relaxed p-3 focus:outline-hidden resize-y min-h-20 text-foreground"
        rows={lineCount}
        value={draftContent}
        oninput={(e) => { draftContent = (e.target as HTMLTextAreaElement).value; }}
      ></textarea>
    {:else if highlightedHtml}
      <div class="shiki-wrapper overflow-x-auto">
        {@html highlightedHtml}
      </div>
    {:else}
      <pre class="font-mono text-xs leading-relaxed p-3 text-foreground whitespace-pre-wrap">{filetype === 'json' ? formatJson(fv.content) : fv.content}</pre>
    {/if}
  </div>
</div>

<style>
  .shiki-wrapper :global(pre.shiki) {
    margin: 0;
    padding: 0.75rem 1rem;
    background: transparent !important;
    font-size: 0.75rem;
    line-height: 1.5;
  }

  .shiki-wrapper :global(pre.shiki code .line) {
    display: inline-block;
    width: 100%;
    padding: 0 0.25rem;
  }

  .shiki-wrapper :global(pre.shiki code .line:hover) {
    background: var(--muted-hover, rgba(128, 128, 128, 0.1));
  }
</style>
