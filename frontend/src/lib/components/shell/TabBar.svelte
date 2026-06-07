<script lang="ts">
  import { tabStore } from '../../stores/tabs.svelte.ts';

  let isActive = $derived((id: string) => id === tabStore.activeId);

  // Drag reorder state
  let dragIndex = $state<number | null>(null);
  let dropIndex = $state<number | null>(null);

  function onDragStart(e: DragEvent, index: number) {
    dragIndex = index;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
    }
  }

  function onDragOver(e: DragEvent, index: number) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dropIndex = index;
  }

  function onDrop(e: DragEvent, index: number) {
    e.preventDefault();
    if (dragIndex !== null && dragIndex !== index) {
      tabStore.reorder(dragIndex, index);
    }
    dragIndex = null;
    dropIndex = null;
  }

  function onDragEnd() {
    dragIndex = null;
    dropIndex = null;
  }
</script>

<div class="bg-background-2 border-b border-line-2">
  <div class="flex items-end px-2 pt-1" aria-label="Tabs" role="tablist" aria-orientation="horizontal">
    {#each tabStore.tabs as tab, i (tab.id)}
      {#if i > 0 && !isActive(tab.id) && !isActive(tabStore.tabs[i - 1].id)}
        <span class="self-center text-line-2 select-none">|</span>
      {/if}
      <button
        type="button"
        role="tab"
        aria-selected={isActive(tab.id)}
        draggable="true"
        ondragstart={(e) => onDragStart(e, i)}
        ondragover={(e) => onDragOver(e, i)}
        ondrop={(e) => onDrop(e, i)}
        ondragend={onDragEnd}
        class="-mb-px py-2 px-4 inline-flex items-center gap-x-2 text-sm font-medium text-center focus:outline-hidden disabled:opacity-50 disabled:pointer-events-none max-w-48 min-w-0 group transition-opacity
          {isActive(tab.id)
            ? 'bg-layer border border-line-2 border-b-transparent rounded-t-lg text-foreground z-10'
            : 'bg-transparent border border-transparent text-muted-foreground-1 hover:text-foreground'}
          {dragIndex === i ? 'opacity-50' : ''}
          {dropIndex === i && dragIndex !== i ? 'border-l-primary' : ''}"
        onclick={() => tabStore.activate(tab.id)}
      >
        <span class="truncate">{tab.title}</span>
        {#if tabStore.tabs.length > 1}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <span
            role="button"
            tabindex="0"
            class="shrink-0 size-4 inline-flex items-center justify-center rounded-sm hover:bg-surface cursor-pointer
              {isActive(tab.id) ? 'text-foreground opacity-100' : 'text-muted-foreground opacity-0 group-hover:opacity-100 hover:text-foreground'}"
            onclick={(e: MouseEvent) => { e.stopPropagation(); tabStore.close(tab.id); }}
            onkeydown={(e: KeyboardEvent) => { if (e.key === 'Enter') { e.stopPropagation(); tabStore.close(tab.id); } }}
            aria-label="Close tab"
            title="Close tab"
          >
            <svg xmlns="http://www.w3.org/2000/svg" class="size-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18"></path>
              <path d="m6 6 12 12"></path>
            </svg>
          </span>
        {/if}
      </button>
    {/each}

    <span class="self-center text-line-2 select-none">|</span>
    <button
      type="button"
      class="shrink-0 size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover mb-0.5"
      onclick={() => tabStore.add()}
      aria-label="New tab"
      title="New tab"
    >
      <svg xmlns="http://www.w3.org/2000/svg" class="size-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M5 12h14"></path>
        <path d="M12 5v14"></path>
      </svg>
    </button>
  </div>
</div>
