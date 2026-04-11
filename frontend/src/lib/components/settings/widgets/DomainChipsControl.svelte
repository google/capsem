<script lang="ts">
  import type { SettingsLeaf, SettingValue } from '../../../types/settings';
  import X from 'phosphor-svelte/lib/X';

  let { leaf, disabled = false, onchange }: {
    leaf: SettingsLeaf;
    disabled?: boolean;
    onchange: (value: SettingValue) => void;
  } = $props();

  let inputValue = $state('');

  let domains = $derived(
    String(leaf.effective_value).split(',').map(d => d.trim()).filter(d => d.length > 0)
  );

  function removeDomain(domain: string) {
    const updated = domains.filter(d => d !== domain);
    onchange(updated.join(', '));
  }

  function addDomain() {
    const input = inputValue.trim();
    if (!input) return;
    const current = [...domains];
    const newDomains = input.split(',').map(d => d.trim()).filter(d => d.length > 0);
    for (const d of newDomains) {
      if (!current.includes(d)) current.push(d);
    }
    inputValue = '';
    onchange(current.join(', '));
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault();
      addDomain();
    }
  }
</script>

<div class="py-2">
  <div class="flex items-center gap-x-2 mb-1">
    <span class="text-sm font-medium text-foreground">{leaf.name}</span>
    {#if leaf.corp_locked}
      <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-destructive/10 text-destructive font-medium">corp</span>
    {/if}
  </div>
  {#if leaf.description}
    <p class="text-xs text-muted-foreground-1 mb-2">{leaf.description}</p>
  {/if}
  <div class="flex flex-wrap gap-1.5 items-center">
    {#each domains as domain (domain)}
      <span class="inline-flex items-center gap-x-1 bg-muted text-foreground text-xs px-2 py-1 rounded-md font-mono">
        {domain}
        {#if !disabled}
          <button
            type="button"
            class="text-muted-foreground-1 hover:text-foreground transition-colors"
            onclick={() => removeDomain(domain)}
            title="Remove {domain}"
          >
            <X size={12} weight="bold" />
          </button>
        {/if}
      </span>
    {/each}
    {#if !disabled}
      <input
        type="text"
        class="py-1 px-2 text-xs rounded-md border border-line-2 bg-layer text-foreground font-mono
          focus:outline-hidden focus:border-primary w-40"
        placeholder="add domain..."
        bind:value={inputValue}
        onkeydown={handleKeydown}
      />
    {/if}
  </div>
</div>
