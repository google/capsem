<script lang="ts">
  import { onMount } from 'svelte';
  import { networkStore } from '../stores/network.svelte';
  import { getNetworkPolicy } from '../api';
  import { queryAll } from '../db';
  import { NET_EVENTS_SQL, NET_EVENTS_SEARCH_SQL } from '../sql';
  import type { NetworkPolicyResponse, NetEvent } from '../types';

  let search = $state('');
  let searchResults = $state<NetEvent[] | null>(null);
  let policy = $state<NetworkPolicyResponse | null>(null);
  let policyExpanded = $state(false);
  let allowExpanded = $state(false);
  let blockExpanded = $state(false);
  let searchTimeout: ReturnType<typeof setTimeout> | null = null;

  onMount(async () => {
    try {
      policy = await getNetworkPolicy();
    } catch (e) {
      console.error('Failed to load network policy:', e);
    }
  });

  // Debounced SQL search.
  function onSearchInput() {
    if (searchTimeout) clearTimeout(searchTimeout);
    const q = search.trim();
    if (!q) {
      searchResults = null;
      return;
    }
    searchTimeout = setTimeout(async () => {
      try {
        searchResults = await queryAll<NetEvent>(NET_EVENTS_SEARCH_SQL, [q, q, q, q, 200]);
      } catch {
        searchResults = [];
      }
    }, 300);
  }

  const displayEvents = $derived(searchResults ?? networkStore.events);

  function timeAgo(epochSec: number): string {
    const diffSec = Math.floor(Date.now() / 1000 - epochSec);
    if (diffSec < 60) return `${diffSec}s ago`;
    if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
    if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h ago`;
    return `${Math.floor(diffSec / 86400)}d ago`;
  }
</script>

<div class="flex h-full flex-col overflow-hidden">
  <div class="flex items-center gap-2 border-b border-base-300 bg-base-200 px-3 py-1.5">
    <span class="text-xs font-semibold">Network</span>
  </div>
  <div class="flex-1 overflow-auto p-4">
    <!-- Summary cards (SQL-driven) -->
    <div class="grid grid-cols-3 gap-3 mb-4">
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Total calls</div>
        <div class="mt-1 text-xl font-semibold tabular-nums">{networkStore.totalCalls}</div>
      </div>
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Allowed</div>
        <div class="mt-1 text-xl font-semibold tabular-nums">{networkStore.allowedCount}</div>
      </div>
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Denied</div>
        <div class="mt-1 text-xl font-semibold tabular-nums">{networkStore.deniedCount}</div>
      </div>
    </div>

    <!-- Active Policy -->
    {#if policy}
      <div class="mb-4 rounded-lg border border-base-300 overflow-hidden">
        <button
          class="flex items-center justify-between w-full px-3 py-2 bg-base-200/50 cursor-pointer"
          onclick={() => policyExpanded = !policyExpanded}
        >
          <div class="flex items-center gap-2">
            <span class="text-[10px] text-base-content/40">{policyExpanded ? '\u25BC' : '\u25B6'}</span>
            <span class="text-xs font-semibold">Active Policy</span>
            <span class="badge badge-xs {policy.default_action === 'deny' ? 'badge-secondary' : 'badge-info'}">
              default: {policy.default_action}
            </span>
            {#if policy.corp_managed}
              <span class="badge badge-xs badge-warning">corp managed</span>
            {/if}
            {#if policy.conflicts.length > 0}
              <span class="badge badge-xs badge-secondary">{policy.conflicts.length} conflict{policy.conflicts.length === 1 ? '' : 's'}</span>
            {/if}
          </div>
          <span class="text-[10px] text-base-content/40">{policy.allow.length} allowed, {policy.block.length} blocked</span>
        </button>
        {#if policyExpanded}
          <div class="border-t border-base-300 space-y-0">
            {#if policy.conflicts.length > 0}
              <div class="px-3 py-2 border-b border-base-300/50">
                <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider mb-1">Conflicts (block wins)</div>
                <div class="flex flex-wrap gap-1">
                  {#each policy.conflicts as domain}
                    <span class="badge badge-xs badge-secondary font-mono">{domain}</span>
                  {/each}
                </div>
              </div>
            {/if}
            <!-- Allowed domains group -->
            <div class="border-b border-base-300/50">
              <button
                class="flex items-center justify-between w-full px-3 py-1.5 cursor-pointer hover:bg-base-200/30"
                onclick={() => allowExpanded = !allowExpanded}
              >
                <div class="flex items-center gap-2">
                  <span class="text-[10px] text-base-content/40">{allowExpanded ? '\u25BC' : '\u25B6'}</span>
                  <span class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Allowed domains</span>
                </div>
                <span class="badge badge-xs badge-info">{policy.allow.length}</span>
              </button>
              {#if allowExpanded}
                <div class="flex flex-wrap gap-1 px-3 pb-2 max-h-32 overflow-auto">
                  {#each policy.allow as domain}
                    <span class="badge badge-xs badge-info font-mono">{domain}</span>
                  {:else}
                    <span class="text-xs text-base-content/30 italic">None</span>
                  {/each}
                </div>
              {/if}
            </div>
            <!-- Blocked domains group -->
            <div>
              <button
                class="flex items-center justify-between w-full px-3 py-1.5 cursor-pointer hover:bg-base-200/30"
                onclick={() => blockExpanded = !blockExpanded}
              >
                <div class="flex items-center gap-2">
                  <span class="text-[10px] text-base-content/40">{blockExpanded ? '\u25BC' : '\u25B6'}</span>
                  <span class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Blocked domains</span>
                </div>
                <span class="badge badge-xs badge-secondary">{policy.block.length}</span>
              </button>
              {#if blockExpanded}
                <div class="flex flex-wrap gap-1 px-3 pb-2 max-h-32 overflow-auto">
                  {#each policy.block as domain}
                    <span class="badge badge-xs badge-secondary font-mono">{domain}</span>
                  {:else}
                    <span class="text-xs text-base-content/30 italic">None</span>
                  {/each}
                </div>
              {/if}
            </div>
          </div>
        {/if}
      </div>
    {/if}

    <!-- Search (SQL-driven) -->
    <div class="mb-3 flex items-center gap-2">
      <input
        type="text"
        class="input input-sm input-bordered w-full text-xs"
        placeholder="Search by domain, method, path, or rule..."
        bind:value={search}
        oninput={onSearchInput}
      />
      {#if searchResults !== null}
        <span class="text-[10px] text-base-content/40 whitespace-nowrap">{searchResults.length} result{searchResults.length === 1 ? '' : 's'}</span>
      {/if}
    </div>

    <!-- Event log -->
    <div class="rounded-lg border border-base-300 overflow-hidden">
      <table class="table table-xs w-full">
        <thead>
          <tr class="bg-base-200/50">
            <th class="text-[10px] font-semibold uppercase tracking-wider">Time</th>
            <th class="text-[10px] font-semibold uppercase tracking-wider">Domain</th>
            <th class="text-[10px] font-semibold uppercase tracking-wider">Request</th>
            <th class="text-[10px] font-semibold uppercase tracking-wider text-right">Status</th>
            <th class="text-[10px] font-semibold uppercase tracking-wider text-right">Decision</th>
          </tr>
        </thead>
        <tbody>
          {#each displayEvents as event}
            <tr class="hover:bg-base-200/30 border-t border-base-300/50">
              <td class="text-[11px] text-base-content/50 whitespace-nowrap tabular-nums">{timeAgo(event.timestamp)}</td>
              <td class="text-[11px] font-mono max-w-[240px] truncate" title={event.domain}>{event.domain}</td>
              <td class="text-[11px] font-mono text-base-content/60 max-w-[300px] truncate">
                {#if event.method}
                  <span class="font-semibold text-base-content/80">{event.method}</span>
                  <span class="ml-1">{event.path ?? ''}</span>
                {:else}
                  <span class="text-base-content/30">--</span>
                {/if}
              </td>
              <td class="text-[11px] text-right tabular-nums">
                {#if event.status_code}
                  <span class={event.status_code < 400 ? 'text-info' : 'text-secondary'}>{event.status_code}</span>
                {:else}
                  <span class="text-base-content/30">--</span>
                {/if}
              </td>
              <td class="text-right">
                {#if event.decision === 'allowed'}
                  <span class="badge badge-xs badge-info">allowed</span>
                {:else if event.decision === 'denied'}
                  <span class="badge badge-xs badge-secondary">denied</span>
                {:else}
                  <span class="badge badge-xs badge-warning">error</span>
                {/if}
              </td>
            </tr>
          {:else}
            <tr>
              <td colspan="5" class="text-center text-xs text-base-content/40 py-8">
                {#if search.trim()}
                  No events matching "{search}"
                {:else}
                  No network events recorded
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
</div>
