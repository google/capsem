<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { logsStore } from '../stores/logs.svelte';
  import { getVmState } from '../api';
  import type { LogLevel, TransitionEntry } from '../types';

  let scrollContainer: HTMLDivElement;
  let timeline = $state<TransitionEntry[]>([]);
  let totalDuration = $state(0);

  const levels: LogLevel[] = ['error', 'warn', 'info', 'debug'];

  function levelClass(level: string): string {
    switch (level) {
      case 'ERROR': return 'text-denied';
      case 'WARN': return 'text-caution';
      case 'DEBUG': return 'text-base-content/40';
      default: return 'text-base-content/60';
    }
  }

  function formatTime(ts: string): string {
    try {
      const d = new Date(ts);
      return d.toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' });
    } catch {
      return ts.slice(11, 19);
    }
  }

  function shortTarget(target: string): string {
    const parts = target.split('::');
    return parts.length > 1 ? parts.slice(1).join('::') : target;
  }

  async function loadTimeline() {
    try {
      const state = await getVmState();
      timeline = state.history;
      totalDuration = state.history.reduce((sum, t) => sum + t.duration_ms, 0);
    } catch {
      timeline = [];
    }
  }

  async function scrollToBottom() {
    await tick();
    if (scrollContainer && logsStore.autoScroll) {
      scrollContainer.scrollTop = scrollContainer.scrollHeight;
    }
  }

  function handleScroll() {
    if (!scrollContainer) return;
    const atBottom = scrollContainer.scrollHeight - scrollContainer.scrollTop - scrollContainer.clientHeight < 40;
    logsStore.autoScroll = atBottom;
  }

  onMount(() => {
    loadTimeline();
  });

  $effect(() => {
    // React to new filtered entries
    logsStore.filteredEntries;
    scrollToBottom();
  });
</script>

<div class="flex flex-col h-full overflow-hidden">
  <!-- Boot Timeline (collapsible) -->
  {#if logsStore.selectedSession === null && timeline.length > 0}
    <div class="border-b border-base-300 px-4 py-3">
      <h3 class="text-xs font-semibold text-base-content/50 uppercase tracking-wider mb-2">Boot Timeline</h3>
      <div class="flex items-center gap-0.5 h-6">
        {#each timeline as t}
          {@const pct = totalDuration > 0 ? Math.max((t.duration_ms / totalDuration) * 100, 2) : 0}
          <div
            class="h-full bg-allowed/20 rounded-sm relative group"
            style:width="{pct}%"
            title="{t.from} -> {t.to} ({t.trigger}) {t.duration_ms.toFixed(0)}ms"
          >
            <div
              class="absolute inset-0 bg-allowed rounded-sm"
              style:width="100%"
              style:opacity="0.6"
            ></div>
            <div class="absolute inset-0 flex items-center justify-center text-[9px] font-mono text-base-content/70 z-10 whitespace-nowrap overflow-hidden">
              {#if pct > 8}
                {t.to} {t.duration_ms.toFixed(0)}ms
              {/if}
            </div>
          </div>
        {/each}
      </div>
      <div class="flex gap-4 mt-1 text-[10px] text-base-content/40">
        {#each timeline as t}
          <span>{t.from} &rarr; {t.to}: {t.duration_ms.toFixed(0)}ms</span>
        {/each}
      </div>
    </div>
  {/if}

  <!-- Session selector + Filter bar -->
  <div class="flex items-center gap-3 px-4 py-2 border-b border-base-300 bg-base-200/50">
    <div class="flex items-center gap-2">
      <span class="text-xs text-base-content/50">Session:</span>
      <select
        class="select select-xs bg-base-100"
        onchange={(e) => {
          const v = (e.target as HTMLSelectElement).value;
          if (v === '__live__') logsStore.switchToLive();
          else logsStore.loadSession(v);
        }}
      >
        <option value="__live__" selected={logsStore.selectedSession === null}>
          Current (live)
        </option>
        {#each logsStore.sessions as s}
          <option value={s.session_id} selected={logsStore.selectedSession === s.session_id}>
            {s.session_id} ({s.entry_count})
          </option>
        {/each}
      </select>
    </div>

    <div class="flex items-center gap-2">
      <span class="text-xs text-base-content/50">Level:</span>
      <select
        class="select select-xs bg-base-100"
        onchange={(e) => logsStore.setFilter((e.target as HTMLSelectElement).value as LogLevel)}
      >
        {#each levels as l}
          <option value={l} selected={logsStore.filterLevel === l}>
            {l.charAt(0).toUpperCase() + l.slice(1)}
          </option>
        {/each}
      </select>
    </div>

    <span class="text-xs text-base-content/40">
      {logsStore.filteredEntries.length} entries
    </span>

    {#if logsStore.errorCount > 0}
      <span class="badge badge-sm bg-denied/15 text-denied">
        {logsStore.errorCount} errors
      </span>
    {/if}

    <div class="flex-1"></div>

    <button
      class="btn btn-ghost btn-xs"
      onclick={() => logsStore.clear()}
    >
      Clear
    </button>

    {#if !logsStore.autoScroll}
      <button
        class="btn btn-ghost btn-xs"
        onclick={() => { logsStore.autoScroll = true; scrollToBottom(); }}
      >
        Jump to bottom
      </button>
    {/if}
  </div>

  <!-- Log entries -->
  <div
    class="flex-1 overflow-y-auto font-mono text-xs"
    bind:this={scrollContainer}
    onscroll={handleScroll}
  >
    {#if logsStore.filteredEntries.length === 0}
      <div class="flex items-center justify-center h-full text-base-content/30">
        {logsStore.selectedSession ? 'No log entries found' : 'Waiting for log events...'}
      </div>
    {:else}
      <table class="w-full">
        <tbody>
          {#each logsStore.filteredEntries as entry}
            <tr class="hover:bg-base-200/50 border-b border-base-200/30">
              <td class="px-2 py-0.5 text-base-content/40 whitespace-nowrap w-20">
                {formatTime(entry.timestamp)}
              </td>
              <td class="px-1 py-0.5 whitespace-nowrap w-12 {levelClass(entry.level)} font-semibold">
                {entry.level}
              </td>
              <td class="px-1 py-0.5 text-base-content/40 whitespace-nowrap w-40 max-w-40 truncate">
                {shortTarget(entry.target)}
              </td>
              <td class="px-2 py-0.5 text-base-content/80 break-all">
                {entry.message}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
</div>
