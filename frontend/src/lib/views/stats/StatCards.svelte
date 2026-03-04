<script lang="ts">
  interface StatCard {
    label: string;
    value: string | number;
    sub?: string;
    colorClass?: string;
  }

  let { cards }: { cards: StatCard[] } = $props();
</script>

<div class="flex gap-3 px-3 py-2">
  {#each cards as card}
    <div class="stat p-2 bg-base-200/40 rounded-lg flex-1 min-w-0">
      <div class="stat-title text-[10px] uppercase tracking-wider text-base-content/40">{card.label}</div>
      <div class="stat-value text-lg font-bold {card.colorClass ?? ''}">{card.value}</div>
      {#if card.sub}
        <div class="stat-desc text-[10px] text-base-content/30">{card.sub}</div>
      {/if}
    </div>
  {/each}
</div>
