<script lang="ts">
  import type { SettingsLeaf, SettingValue } from '../../../types/settings';
  import Eye from 'phosphor-svelte/lib/Eye';
  import EyeSlash from 'phosphor-svelte/lib/EyeSlash';
  import ArrowSquareOut from 'phosphor-svelte/lib/ArrowSquareOut';

  let { leaf, disabled = false, onchange }: {
    leaf: SettingsLeaf;
    disabled?: boolean;
    onchange: (value: SettingValue) => void;
  } = $props();

  let revealed = $state(false);
  let value = $derived(String(leaf.effective_value));
  let isEmpty = $derived(value.length === 0);
  let hasPrefixWarning = $derived(
    leaf.metadata.prefix && value.length > 0 && !value.startsWith(leaf.metadata.prefix)
  );
</script>

<div class="flex items-start justify-between gap-x-4 py-2">
  <div class="flex-1 min-w-0">
    <div class="flex items-center gap-x-2">
      <span class="text-sm font-medium text-foreground">{leaf.name}</span>
      {#if isEmpty && !disabled}
        <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-warning/10 text-warning font-medium">required</span>
      {/if}
      {#if leaf.corp_locked}
        <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-destructive/10 text-destructive font-medium">corp</span>
      {/if}
      {#if leaf.metadata.docs_url}
        <a
          href={leaf.metadata.docs_url}
          target="_blank"
          rel="noopener"
          class="text-primary hover:text-primary-hover"
          title="Get API key"
        >
          <ArrowSquareOut size={14} />
        </a>
      {/if}
    </div>
    {#if leaf.description}
      <p class="text-xs text-muted-foreground-1 mt-0.5">{leaf.description}</p>
    {/if}
  </div>
  <div class="flex flex-col items-end gap-y-1 shrink-0">
    <div class="flex items-center gap-x-1">
      <input
        type={revealed ? 'text' : 'password'}
        class="py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground font-mono
          focus:outline-hidden focus:border-primary w-64
          {disabled ? 'opacity-50 cursor-not-allowed' : ''}"
        value={value}
        placeholder={leaf.metadata.prefix ? `${leaf.metadata.prefix}...` : ''}
        {disabled}
        oninput={(e) => onchange((e.target as HTMLInputElement).value)}
      />
      <button
        type="button"
        class="p-2 rounded-md text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover transition-colors"
        onclick={() => revealed = !revealed}
        title={revealed ? 'Hide' : 'Show'}
      >
        {#if revealed}
          <EyeSlash size={16} />
        {:else}
          <Eye size={16} />
        {/if}
      </button>
    </div>
    {#if hasPrefixWarning}
      <span class="text-xs text-warning">Token should start with {leaf.metadata.prefix}</span>
    {/if}
  </div>
</div>
