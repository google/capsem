<script lang="ts">
  import Play from 'phosphor-svelte/lib/Play';
  import CaretDown from 'phosphor-svelte/lib/CaretDown';
  import * as api from '../../api';
  import {
    mockPresetQueries, validateSelectOnly, executeMockQuery,
  } from '../../mock.ts';
  import type { MockQueryResult } from '../../mock.ts';

  let { vmId }: { vmId: string } = $props();

  let sql = $state('');
  let result = $state<MockQueryResult | null>(null);
  let error = $state<string | null>(null);
  let presetOpen = $state(false);
  let running = $state(false);

  async function runQuery() {
    error = null;
    result = null;

    const validationError = validateSelectOnly(sql);
    if (validationError) {
      error = validationError;
      return;
    }

    running = true;
    try {
      result = await api.inspectQuery(vmId, sql);
    } catch (e) {
      error = e instanceof Error ? e.message : 'Query failed';
    } finally {
      running = false;
    }
  }

  async function selectPreset(preset: { label: string; sql: string }) {
    sql = preset.sql;
    presetOpen = false;
    error = null;
    running = true;
    try {
      result = await api.inspectQuery(vmId, preset.sql);
    } catch (e) {
      error = e instanceof Error ? e.message : 'Query failed';
    } finally {
      running = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      runQuery();
    }
  }

  function onClickOutside(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (!target.closest('[data-preset-menu]')) {
      presetOpen = false;
    }
  }

  // Sort columns for display
  let sortColumn = $state<string | null>(null);
  let sortAsc = $state(true);

  let sortedRows = $derived.by(() => {
    if (!result || !sortColumn) return result?.rows ?? [];
    const col = sortColumn;
    return [...result.rows].sort((a, b) => {
      const va = a[col];
      const vb = b[col];
      if (va == null && vb == null) return 0;
      if (va == null) return 1;
      if (vb == null) return -1;
      if (typeof va === 'number' && typeof vb === 'number') {
        return sortAsc ? va - vb : vb - va;
      }
      const sa = String(va);
      const sb = String(vb);
      return sortAsc ? sa.localeCompare(sb) : sb.localeCompare(sa);
    });
  });

  function toggleSort(col: string) {
    if (sortColumn === col) {
      sortAsc = !sortAsc;
    } else {
      sortColumn = col;
      sortAsc = true;
    }
  }
</script>

<svelte:document onclick={onClickOutside} />

<div class="flex flex-col h-full">
  <!-- SQL editor area -->
  <div class="border-b border-line-2 bg-layer">
    <!-- Toolbar -->
    <div class="flex items-center gap-x-2 px-4 py-2 border-b border-line-2">
      <button
        type="button"
        class="inline-flex items-center gap-x-1.5 px-3 py-1.5 text-sm font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover disabled:opacity-40"
        onclick={runQuery}
        disabled={!sql.trim()}
        title="Run query (Cmd+Enter)"
      >
        <Play size={14} weight="fill" />
        Run
      </button>

      <!-- Preset dropdown -->
      <div class="relative" data-preset-menu>
        <button
          type="button"
          class="inline-flex items-center gap-x-1.5 px-3 py-1.5 text-sm rounded-lg bg-layer border border-line-2 text-foreground hover:bg-muted-hover"
          onclick={(e: MouseEvent) => { e.stopPropagation(); presetOpen = !presetOpen; }}
        >
          Presets
          <CaretDown size={12} />
        </button>
        {#if presetOpen}
          <div class="absolute start-0 top-full mt-1 w-72 bg-dropdown border border-dropdown-border rounded-xl shadow-lg z-50">
            <div class="p-1">
              {#each mockPresetQueries as preset}
                <button
                  type="button"
                  class="w-full flex flex-col gap-y-0.5 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover text-left"
                  onclick={() => selectPreset(preset)}
                >
                  <span class="font-medium">{preset.label}</span>
                  <span class="font-mono text-xs text-muted-foreground truncate">{preset.sql}</span>
                </button>
              {/each}
            </div>
          </div>
        {/if}
      </div>

      <span class="text-xs text-muted-foreground ml-auto">Cmd+Enter to run</span>
    </div>

    <!-- SQL textarea -->
    <textarea
      class="w-full px-4 py-3 font-mono text-sm bg-background text-foreground resize-none focus:outline-none"
      rows="4"
      placeholder="SELECT * FROM event_log LIMIT 10"
      bind:value={sql}
      onkeydown={handleKeydown}
      spellcheck={false}
    ></textarea>
  </div>

  <!-- Results area -->
  <div class="flex-1 overflow-auto">
    {#if error}
      <div class="m-4 p-4 bg-destructive/10 border border-destructive/20 rounded-xl">
        <p class="text-sm text-destructive font-medium">{error}</p>
      </div>
    {:else if result}
      <div class="p-4">
        <div class="mb-2 text-xs text-muted-foreground">
          {result.rows.length} row{result.rows.length !== 1 ? 's' : ''}
        </div>
        <div class="bg-card border border-card-line rounded-xl overflow-hidden">
          <table class="w-full text-sm">
            <thead>
              <tr class="border-b border-card-divider bg-surface">
                {#each result.columns as col}
                  <th
                    class="text-left px-4 py-2 text-muted-foreground font-medium cursor-pointer select-none hover:text-foreground"
                    onclick={() => toggleSort(col)}
                  >
                    {col}
                    {#if sortColumn === col}
                      <span class="text-xs ml-1">{sortAsc ? '\u2191' : '\u2193'}</span>
                    {/if}
                  </th>
                {/each}
              </tr>
            </thead>
            <tbody>
              {#each sortedRows as row}
                <tr class="border-b border-card-divider last:border-0 hover:bg-muted-hover">
                  {#each result.columns as col}
                    <td class="px-4 py-2 font-mono text-xs text-foreground">
                      {row[col] != null ? row[col] : 'NULL'}
                    </td>
                  {/each}
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      </div>
    {:else}
      <div class="flex items-center justify-center h-full">
        <p class="text-muted-foreground">Run a query or select a preset</p>
      </div>
    {/if}
  </div>
</div>
