<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    subtitle?: string;
    dark?: boolean;
    class?: string;
    children: Snippet;
  }

  let { title, subtitle, dark = false, class: cls = "", children }: Props = $props();
</script>

<div class="rounded-2xl border {dark ? 'border-border-dark bg-surface-card-dark' : 'border-border bg-surface-card'} p-8 {cls}">
  <h3 class="text-xl font-bold {dark ? 'text-heading-dark' : 'text-heading'} mb-1">{title}</h3>
  {#if subtitle}
    <p class="text-sm {dark ? 'text-muted-dark' : 'text-muted'} mb-6">{subtitle}</p>
  {/if}
  {@render children()}
</div>
