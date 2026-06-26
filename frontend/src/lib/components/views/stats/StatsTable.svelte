<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    columns,
    rows,
    children,
  }: {
    columns: string[];
    rows: any[];
    children: Snippet<[any]>;
  } = $props();
</script>

<div class="bg-card border border-card-line rounded-xl overflow-hidden mb-6">
  <table class="w-full text-sm">
    <thead>
      <tr class="border-b border-card-divider bg-surface">
        {#each columns as column}
          <th class="text-left px-4 py-2 text-muted-foreground font-medium">{column}</th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each rows as row}
        <tr class="border-b border-card-divider last:border-0">
          {@render children(row)}
        </tr>
      {:else}
        <tr><td class="px-4 py-6 text-center text-muted-foreground" colspan={columns.length}>No rows</td></tr>
      {/each}
    </tbody>
  </table>
</div>
