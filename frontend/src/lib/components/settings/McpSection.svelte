<script lang="ts">
  import { MOCK_MCP_TOOLS, MOCK_MCP_POLICY, MOCK_MCP_SERVERS } from '../../mock-settings';
  import type { McpToolInfo } from '../../types/settings';
  import CaretDown from 'phosphor-svelte/lib/CaretDown';
  import Shield from 'phosphor-svelte/lib/Shield';
  import Wrench from 'phosphor-svelte/lib/Wrench';
  import Globe from 'phosphor-svelte/lib/Globe';

  let expandedGroups = $state<Set<string>>(new Set());

  function toggleGroup(key: string) {
    const next = new Set(expandedGroups);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    expandedGroups = next;
  }

  // Group builtin tools by category
  let httpTools = $derived(MOCK_MCP_TOOLS.filter(t => t.server_name === 'builtin' && t.namespaced_name.startsWith('fetch') || t.namespaced_name.startsWith('grep_http') || t.namespaced_name.startsWith('http_')));
  let snapshotTools = $derived(MOCK_MCP_TOOLS.filter(t => t.server_name === 'builtin' && t.namespaced_name.startsWith('snapshots_')));
</script>

<div class="space-y-6">
  <!-- Policy -->
  <div>
    <h2 class="text-xl font-bold text-foreground mb-1">MCP Servers</h2>
    <p class="text-sm text-muted-foreground-1 mb-4">Model Context Protocol servers available to AI agents inside the sandbox.</p>

    <h3 class="text-base font-semibold text-primary mb-2">
      <Shield size={16} class="inline -mt-0.5 mr-1" />
      Policy
    </h3>
    <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider mb-6">
      <div class="flex items-center justify-between p-4">
        <div>
          <p class="text-sm font-medium text-foreground">Default tool permission</p>
          <p class="text-xs text-muted-foreground-1 mt-0.5">Action when an AI agent calls a tool not in any policy</p>
        </div>
        <select
          class="py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary w-32"
          value={MOCK_MCP_POLICY.default_tool_permission}
        >
          <option value="allow">Allow</option>
          <option value="warn">Warn</option>
          <option value="block">Block</option>
        </select>
      </div>
      {#if MOCK_MCP_POLICY.blocked_servers.length > 0}
        <div class="p-4">
          <p class="text-sm font-medium text-foreground mb-1">Blocked servers</p>
          <div class="flex flex-wrap gap-1.5">
            {#each MOCK_MCP_POLICY.blocked_servers as server}
              <span class="bg-destructive/10 text-destructive text-xs px-2 py-1 rounded-md font-mono">{server}</span>
            {/each}
          </div>
        </div>
      {/if}
    </div>
  </div>

  <!-- Built-in Tools -->
  <div>
    <h3 class="text-base font-semibold text-primary mb-2">
      <Wrench size={16} class="inline -mt-0.5 mr-1" />
      Built-in Tools
    </h3>

    {#snippet toolGroup(title: string, tools: McpToolInfo[], groupKey: string)}
      {@const isExpanded = expandedGroups.has(groupKey)}
      <div class="bg-card border border-card-line rounded-xl overflow-hidden mb-3">
        <button
          type="button"
          class="w-full flex items-center justify-between px-4 py-3 bg-background-1 hover:bg-muted-hover transition-colors"
          onclick={() => toggleGroup(groupKey)}
        >
          <div class="flex items-center gap-x-2">
            <span class="text-sm font-semibold text-foreground">{title}</span>
            <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1">{tools.length}</span>
          </div>
          <CaretDown size={14} class="text-muted-foreground-1 transition-transform duration-300 {isExpanded ? 'rotate-180' : ''}" />
        </button>
        {#if isExpanded}
          <div class="divide-y divide-card-divider">
            {#each tools as tool (tool.namespaced_name)}
              <div class="px-4 py-2.5 flex items-start justify-between gap-x-3">
                <div class="min-w-0">
                  <div class="flex items-center gap-x-2">
                    <span class="text-sm font-medium text-foreground font-mono">{tool.original_name}</span>
                    {#if tool.annotations?.read_only_hint}
                      <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-green-100 dark:bg-green-900/30 text-green-700 dark:text-green-400">read-only</span>
                    {/if}
                    {#if tool.annotations?.destructive_hint}
                      <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-destructive/10 text-destructive">destructive</span>
                    {/if}
                  </div>
                  <p class="text-xs text-muted-foreground-1 mt-0.5 line-clamp-2">{tool.description}</p>
                </div>
                <span class="text-xs text-green-600 dark:text-green-400 shrink-0 mt-0.5">
                  {tool.approved ? 'Allowed' : 'Blocked'}
                </span>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {/snippet}

    {@render toolGroup('HTTP', httpTools, 'tools-http')}
    {@render toolGroup('Snapshots', snapshotTools, 'tools-snapshots')}
  </div>

  <!-- External Servers -->
  <div>
    <h3 class="text-base font-semibold text-primary mb-2">
      <Globe size={16} class="inline -mt-0.5 mr-1" />
      External Servers
    </h3>
    {#if MOCK_MCP_SERVERS.length === 0}
      <div class="bg-card border border-card-line rounded-xl p-6 text-center">
        <p class="text-sm text-muted-foreground-1">No external MCP servers configured.</p>
        <p class="text-xs text-muted-foreground-1 mt-1">Add servers in user.toml or via the gateway API.</p>
      </div>
    {:else}
      {#each MOCK_MCP_SERVERS as server (server.name)}
        <div class="bg-card border border-card-line rounded-xl p-4 mb-3">
          <div class="flex items-center justify-between">
            <div>
              <span class="text-sm font-semibold text-foreground">{server.name}</span>
              <span class="text-xs text-muted-foreground-1 ml-2">{server.transport}</span>
            </div>
            <span class="text-xs {server.healthy ? 'text-green-600 dark:text-green-400' : 'text-destructive'}">
              {server.healthy ? 'Healthy' : 'Unhealthy'}
            </span>
          </div>
          {#if server.url}
            <p class="text-xs text-muted-foreground-1 font-mono mt-1">{server.url}</p>
          {/if}
        </div>
      {/each}
    {/if}
  </div>
</div>
