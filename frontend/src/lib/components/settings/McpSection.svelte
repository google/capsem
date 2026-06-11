<script lang="ts">
  import { onMount } from 'svelte';
  import { slide } from 'svelte/transition';
  import { mcpStore } from '../../stores/mcp.svelte.ts';
  import type { McpServerInfo, McpToolInfo, ToolPermission } from '../../types';
  import ArrowClockwise from 'phosphor-svelte/lib/ArrowClockwise';
  import CaretDown from 'phosphor-svelte/lib/CaretDown';
  import WarningCircle from 'phosphor-svelte/lib/WarningCircle';

  let { profileId } = $props<{ profileId: string }>();
  let servers = $derived(mcpStore.servers);
  let userServers = $derived(servers.filter(s => s.source !== 'builtin'));
  let builtinServers = $derived(servers.filter(s => s.source === 'builtin'));
  let actionError = $state<string | null>(null);
  let loadedProfileId = $state<string | null>(null);

  // Runtime status lookup by server name
  let runtimeByName = $derived.by(() => {
    const map = new Map<string, McpServerInfo>();
    for (const s of mcpStore.servers) map.set(s.name, s);
    return map;
  });

  // --- Expand/collapse ---
  let expandedGroups = $state<Set<string>>(new Set());

  function toggleGroup(key: string) {
    const next = new Set(expandedGroups);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    expandedGroups = next;
  }

  let saving = $state(false);

  onMount(() => {
    if (profileId) {
      loadedProfileId = profileId;
      void mcpStore.load(profileId);
    }
  });

  $effect(() => {
    if (profileId && profileId !== loadedProfileId) {
      loadedProfileId = profileId;
      void mcpStore.load(profileId);
    }
  });

  async function setToolPermission(tool: McpToolInfo, action: ToolPermission) {
    saving = true;
    actionError = null;
    try {
      await mcpStore.setToolPermission(tool, action);
    } catch (err) {
      actionError = String(err instanceof Error ? err.message : err);
    } finally {
      saving = false;
    }
  }

</script>

{#snippet toolList(tools: McpToolInfo[])}
  <div transition:slide={{ duration: 300 }} class="divide-y divide-card-divider border-t border-card-divider">
    {#each tools as tool (tool.namespaced_name)}
      <div class="px-4 py-3 flex items-start justify-between gap-x-3">
        <div class="min-w-0">
          <div class="flex items-center gap-x-2 flex-wrap">
            <span class="text-sm font-mono text-foreground">{tool.original_name}</span>
            {#if tool.annotations?.read_only_hint}
              <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-primary/10 text-primary">read-only</span>
            {/if}
            {#if tool.annotations?.destructive_hint}
              <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-destructive/10 text-destructive">destructive</span>
            {/if}
            {#if tool.pin_changed}
              <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-warning/10 text-warning inline-flex items-center gap-x-0.5">
                <WarningCircle size={10} /> changed
              </span>
            {/if}
          </div>
          {#if tool.description}
            <p class="text-xs text-muted-foreground-1 mt-1">{tool.description}</p>
          {/if}
          <p class="text-[10px] text-muted-foreground-2 mt-1">
            Permission source: {tool.permission_source}
          </p>
        </div>
        <label class="sr-only" for={`mcp-permission-${tool.namespaced_name}`}>Permission for {tool.original_name}</label>
        <select
          id={`mcp-permission-${tool.namespaced_name}`}
          class="shrink-0 rounded-lg border border-line-2 bg-layer px-2 py-1 text-xs text-foreground disabled:opacity-50"
          value={tool.permission_action}
          disabled={saving}
          onchange={(event) => setToolPermission(tool, event.currentTarget.value as ToolPermission)}
        >
          <option value="allow">Allow</option>
          <option value="ask">Ask</option>
          <option value="block">Block</option>
        </select>
      </div>
    {/each}
  </div>
{/snippet}

<div class="space-y-6">
  <!-- Header -->
  <div class="flex items-center justify-between">
    <div>
      <h2 class="text-xl font-medium text-foreground">MCP Servers</h2>
      <p class="text-sm text-muted-foreground-1 mt-0.5">Model Context Protocol servers available to AI agents inside the sandbox.</p>
    </div>
    <button
      type="button"
      class="p-2 rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover transition-colors disabled:opacity-40"
      title="Refresh tools"
      disabled={mcpStore.loading}
      onclick={() => mcpStore.refresh()}
    >
      <ArrowClockwise size={18} class={mcpStore.loading ? 'animate-spin' : ''} />
    </button>
  </div>

  {#if actionError || mcpStore.error}
    <div class="border border-destructive/40 rounded-lg p-3 text-sm text-destructive-foreground">
      {actionError ?? mcpStore.error}
    </div>
  {/if}

  <!-- Built-in Servers -->
  {#if builtinServers.length > 0}
    <div>
      <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">Built-in</h3>
      {#each builtinServers as server (server.name)}
        {@const runtime = runtimeByName.get(server.name)}
        {@const tools = mcpStore.toolsByServer[server.name] ?? []}
        {@const isExpanded = expandedGroups.has(server.name)}
        <div class="bg-card border border-card-line rounded-xl mb-3 overflow-hidden">
          <div class="flex items-center justify-between px-4 py-3">
            <button
              type="button"
              class="flex items-center gap-x-3 min-w-0 flex-1 text-left"
              onclick={() => toggleGroup(server.name)}
            >
              <span class="text-sm font-semibold text-foreground font-mono truncate">{server.name}</span>
              <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1 shrink-0">{server.is_stdio ? 'stdio' : 'http'}</span>
              {#if runtime}
                <span class="flex items-center gap-x-1 text-[10px] px-1.5 py-0.5 rounded-full shrink-0
                  {runtime.running ? 'bg-primary/10 text-primary' : 'bg-muted text-muted-foreground-1'}">
                  <span class="size-1.5 rounded-full {runtime.running ? 'bg-primary' : 'bg-muted-foreground-1'}"></span>
                  {runtime.running ? 'Running' : 'Stopped'}
                </span>
              {/if}
              {#if tools.length > 0}
                <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1 shrink-0">
                  {tools.length} tool{tools.length === 1 ? '' : 's'}
                </span>
              {/if}
              {#if tools.length > 0}
                <CaretDown size={14} class="text-muted-foreground-1 transition-transform duration-300 shrink-0 {isExpanded ? 'rotate-180' : ''}" />
              {/if}
            </button>
          </div>
          {#if server.has_auth_credential && !isExpanded}
            <div class="px-4 pb-3">
              <p class="text-xs text-muted-foreground-1">Uses brokered credential reference</p>
            </div>
          {/if}
          {#if isExpanded && tools.length > 0}
            {@render toolList(tools)}
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  <!-- External Servers -->
  <div>
    <!-- Server list -->
    {#if userServers.length === 0}
      <div class="bg-card border border-card-line rounded-xl p-6 text-center">
        <p class="text-sm text-muted-foreground-1">No external MCP servers configured.</p>
      </div>
    {:else}
      {#each userServers as server (server.name)}
        {@const runtime = runtimeByName.get(server.name)}
        {@const tools = mcpStore.toolsByServer[server.name] ?? []}
        {@const isExpanded = expandedGroups.has(server.name)}
        <div class="bg-card border border-card-line rounded-xl mb-3 overflow-hidden">
          <div class="flex items-center justify-between px-4 py-3">
            <button
              type="button"
              class="flex items-center gap-x-3 min-w-0 flex-1 text-left"
              onclick={() => toggleGroup(server.name)}
            >
              <span class="text-sm font-semibold text-foreground font-mono truncate">{server.name}</span>
              <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1 shrink-0">{server.is_stdio ? 'stdio' : 'http'}</span>
              {#if runtime}
                <span class="flex items-center gap-x-1 text-[10px] px-1.5 py-0.5 rounded-full shrink-0
                  {runtime.running ? 'bg-primary/10 text-primary' : 'bg-muted text-muted-foreground-1'}">
                  <span class="size-1.5 rounded-full {runtime.running ? 'bg-primary' : 'bg-muted-foreground-1'}"></span>
                  {runtime.running ? 'Running' : 'Stopped'}
                </span>
              {/if}
              {#if tools.length > 0}
                <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1 shrink-0">
                  {tools.length} tool{tools.length === 1 ? '' : 's'}
                </span>
              {/if}
              {#if tools.length > 0}
                <CaretDown size={14} class="text-muted-foreground-1 transition-transform duration-300 shrink-0 {isExpanded ? 'rotate-180' : ''}" />
              {/if}
            </button>
          </div>
          {#if server.url && !isExpanded}
            <div class="px-4 pb-3">
              <p class="text-xs text-muted-foreground-1 font-mono truncate">{server.url}</p>
            </div>
          {/if}
          {#if isExpanded && tools.length > 0}
            {@render toolList(tools)}
          {/if}
        </div>
      {/each}
    {/if}
  </div>
</div>
