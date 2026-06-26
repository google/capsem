<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    title,
    rows,
    columns,
    onrow,
    children,
  }: {
    title: string;
    rows: any[];
    columns: string[];
    onrow: (row: any) => void;
    children: Snippet<[any]>;
  } = $props();
</script>

<section class="mb-6">
  <h3 class="text-sm font-semibold text-foreground mb-2">{title}</h3>
  <div class="bg-card border border-card-line rounded-xl overflow-hidden">
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
          <tr class="border-b border-card-divider last:border-0 hover:bg-muted-hover cursor-pointer" onclick={() => onrow(row)}>
            {@render children(row)}
          </tr>
        {:else}
          <tr><td class="px-4 py-6 text-center text-muted-foreground" colspan={columns.length}>No events</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
</section>
