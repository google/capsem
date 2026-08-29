<script lang="ts">
  import Icon from "./Icon.svelte";
  import { SITE } from "$lib/data";

  interface Props {
    dark?: boolean;
  }

  let { dark = false }: Props = $props();
  let copied = $state(false);

  async function copy() {
    await navigator.clipboard.writeText(SITE.installCmd);
    copied = true;
    setTimeout(() => { copied = false; }, 2000);
  }
</script>

<div class="flex items-center gap-3 rounded-xl border {dark ? 'border-border-dark bg-surface-dark-alt/60' : 'border-border bg-surface-alt/60'} px-5 py-3.5">
  <span class="{dark ? 'text-muted-dark' : 'text-muted'} select-none" aria-hidden="true">$</span>
  <code class="font-mono text-sm {dark ? 'text-heading-dark' : 'text-heading'}">{SITE.installCmd}</code>
  <button
    onclick={copy}
    class="ml-auto shrink-0 rounded-md p-1.5 {dark ? 'text-muted-dark hover:text-heading-dark hover:bg-surface-dark' : 'text-muted hover:text-heading hover:bg-surface'} transition-colors"
    aria-label={copied ? "Copied" : "Copy install command"}
  >
    {#if copied}
      <Icon name="check" class="h-4 w-4 text-green-500" />
    {:else}
      <Icon name="copy" />
    {/if}
  </button>
</div>
