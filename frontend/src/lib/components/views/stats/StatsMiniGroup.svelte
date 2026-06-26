<script lang="ts">
  let {
    title,
    rows,
    nameKey,
  }: {
    title: string;
    rows: any[];
    nameKey: string;
  } = $props();
</script>

<div class="bg-card border border-card-line rounded-xl overflow-hidden">
  <div class="px-4 py-2 border-b border-card-divider bg-surface text-sm font-medium text-foreground">{title}</div>
  <div class="divide-y divide-card-divider">
    {#each rows as row}
      <div class="px-4 py-2 flex items-center justify-between gap-x-3 text-sm">
        <span class="font-mono text-xs text-muted-foreground-1">{row[nameKey]}</span>
        <span class="text-foreground">{row.count}</span>
      </div>
    {:else}
      <div class="px-4 py-4 text-sm text-center text-muted-foreground">No rows</div>
    {/each}
  </div>
</div>
