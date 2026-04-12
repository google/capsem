<script lang="ts">
  import { MOCK_MCP_TOOLS, MOCK_MCP_POLICY } from '../../mock-settings';
  import type { McpToolInfo, McpServerInfo } from '../../types/settings';
  import CaretDown from 'phosphor-svelte/lib/CaretDown';
  import Plus from 'phosphor-svelte/lib/Plus';
  import Trash from 'phosphor-svelte/lib/Trash';
  import X from 'phosphor-svelte/lib/X';

  // --- Server list (mock state, will wire to gateway in Sprint 05) ---
  let servers = $state<McpServerInfo[]>([]);

  // --- Add server form ---
  let showAddForm = $state(false);
  let newName = $state('');
  let newUrl = $state('');
  let newBearerToken = $state('');
  let newHeaders = $state<{ key: string; value: string }[]>([]);

  let canAdd = $derived(newName.trim().length > 0 && newUrl.trim().length > 0);

  function resetForm() {
    newName = '';
    newUrl = '';
    newBearerToken = '';
    newHeaders = [];
    showAddForm = false;
  }

  function addHeader() {
    newHeaders = [...newHeaders, { key: '', value: '' }];
  }

  function removeHeader(index: number) {
    newHeaders = newHeaders.filter((_, i) => i !== index);
  }

  function addServer() {
    if (!canAdd) return;
    servers = [...servers, {
      name: newName.trim(),
      url: newUrl.trim(),
      transport: 'http',
      enabled: true,
      builtin: false,
      tool_count: 0,
      healthy: true,
    }];
    resetForm();
  }

  function removeServer(name: string) {
    servers = servers.filter(s => s.name !== name);
  }

  function toggleServer(name: string) {
    servers = servers.map(s =>
      s.name === name ? { ...s, enabled: !s.enabled } : s,
    );
  }

  // --- Expand/collapse ---
  let expandedGroups = $state<Set<string>>(new Set());

  function toggleGroup(key: string) {
    const next = new Set(expandedGroups);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    expandedGroups = next;
  }

  // Group builtin tools by category
  let httpTools = $derived(MOCK_MCP_TOOLS.filter(t => t.server_name === 'builtin' && (t.namespaced_name.startsWith('fetch') || t.namespaced_name.startsWith('grep_http') || t.namespaced_name.startsWith('http_'))));
  let snapshotTools = $derived(MOCK_MCP_TOOLS.filter(t => t.server_name === 'builtin' && t.namespaced_name.startsWith('snapshots_')));
</script>

<div class="space-y-6">
  <!-- Header -->
  <div>
    <div class="mb-6">
      <h2 class="text-xl font-medium text-foreground">MCP Servers</h2>
      <p class="text-sm text-muted-foreground-1 mt-0.5">Model Context Protocol servers available to AI agents inside the sandbox.</p>
    </div>

    <!-- Policy -->
    <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">Policy</h3>
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
              <span class="bg-destructive/10 text-destructive-foreground text-xs px-2 py-1 rounded-md font-mono">{server}</span>
            {/each}
          </div>
        </div>
      {/if}
    </div>
  </div>

  <!-- Built-in Tools -->
  <div>
    <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2 mt-6">Built-in Tools</h3>

    {#snippet toolGroup(title: string, tools: McpToolInfo[], groupKey: string)}
      {@const isExpanded = expandedGroups.has(groupKey)}
      <div class="bg-card border border-card-line rounded-xl overflow-hidden mb-3">
        <button
          type="button"
          class="w-full flex items-center justify-between px-4 py-3 hover:bg-muted-hover transition-colors"
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
                      <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-primary/10 text-primary">read-only</span>
                    {/if}
                    {#if tool.annotations?.destructive_hint}
                      <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-destructive/10 text-destructive-foreground">destructive</span>
                    {/if}
                  </div>
                  <p class="text-xs text-muted-foreground-1 mt-0.5 line-clamp-2">{tool.description}</p>
                </div>
                <span class="text-xs shrink-0 mt-0.5 {tool.approved ? 'text-primary' : 'text-destructive-foreground'}">
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
    <div class="flex items-center justify-between mb-2 mt-6">
      <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider">External Servers</h3>
      {#if !showAddForm}
        <button
          type="button"
          class="py-1.5 px-3 inline-flex items-center gap-x-1.5 text-xs font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover transition-colors"
          onclick={() => showAddForm = true}
        >
          <Plus size={14} />
          Add server
        </button>
      {/if}
    </div>

    <!-- Add server form -->
    {#if showAddForm}
      <div class="bg-card border border-card-line rounded-xl mb-3">
        <div class="flex items-center justify-between px-4 py-3 border-b border-card-divider">
          <span class="text-sm font-semibold text-foreground">New server</span>
          <button
            type="button"
            class="p-1 rounded-md text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover transition-colors"
            onclick={resetForm}
          >
            <X size={16} />
          </button>
        </div>
        <div class="p-4 space-y-4">
          <!-- Name -->
          <div>
            <label for="mcp-name" class="text-xs font-medium text-foreground block mb-1">Name</label>
            <input
              id="mcp-name"
              type="text"
              class="w-full py-2 px-3 text-sm font-mono rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              placeholder="my-server"
              bind:value={newName}
            />
          </div>
          <!-- URL -->
          <div>
            <label for="mcp-url" class="text-xs font-medium text-foreground block mb-1">URL</label>
            <input
              id="mcp-url"
              type="url"
              class="w-full py-2 px-3 text-sm font-mono rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              placeholder="https://mcp.example.com/v1"
              bind:value={newUrl}
            />
          </div>
          <!-- Bearer token -->
          <div>
            <label for="mcp-token" class="text-xs font-medium text-foreground block mb-1">
              Bearer token <span class="text-muted-foreground-1 font-normal">(optional)</span>
            </label>
            <input
              id="mcp-token"
              type="password"
              class="w-full py-2 px-3 text-sm font-mono rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              placeholder="tok_..."
              bind:value={newBearerToken}
            />
          </div>
          <!-- Custom headers -->
          <div>
            <div class="flex items-center justify-between mb-1">
              <span class="text-xs font-medium text-foreground">
                Custom headers <span class="text-muted-foreground-1 font-normal">(optional)</span>
              </span>
              <button
                type="button"
                class="text-xs text-primary hover:text-primary-hover transition-colors"
                onclick={addHeader}
              >
                + Add header
              </button>
            </div>
            {#each newHeaders as header, i (i)}
              <div class="flex items-center gap-x-2 mb-2">
                <input
                  type="text"
                  class="flex-1 py-2 px-3 text-sm font-mono rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
                  placeholder="Header-Name"
                  bind:value={header.key}
                />
                <span class="text-muted-foreground-1 text-sm">:</span>
                <input
                  type="text"
                  class="flex-1 py-2 px-3 text-sm font-mono rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
                  placeholder="value"
                  bind:value={header.value}
                />
                <button
                  type="button"
                  class="p-1.5 rounded-md text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover transition-colors"
                  onclick={() => removeHeader(i)}
                >
                  <X size={14} />
                </button>
              </div>
            {/each}
          </div>
          <!-- Actions -->
          <div class="flex items-center justify-end gap-x-2 pt-2">
            <button
              type="button"
              class="py-2 px-4 text-sm font-medium rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover transition-colors"
              onclick={resetForm}
            >
              Cancel
            </button>
            <button
              type="button"
              class="py-2 px-4 text-sm font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              disabled={!canAdd}
              onclick={addServer}
            >
              Add Server
            </button>
          </div>
        </div>
      </div>
    {/if}

    <!-- Server list -->
    {#if servers.length === 0 && !showAddForm}
      <div class="bg-card border border-card-line rounded-xl p-6 text-center">
        <p class="text-sm text-muted-foreground-1">No external MCP servers configured.</p>
        <button
          type="button"
          class="mt-2 text-sm text-primary hover:text-primary-hover transition-colors"
          onclick={() => showAddForm = true}
        >
          Add your first server
        </button>
      </div>
    {:else}
      {#each servers as server (server.name)}
        <div class="bg-card border border-card-line rounded-xl mb-3 overflow-hidden">
          <div class="flex items-center justify-between px-4 py-3">
            <div class="flex items-center gap-x-3 min-w-0">
              <span class="text-sm font-semibold text-foreground font-mono truncate">{server.name}</span>
              <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1 shrink-0">{server.transport}</span>
              {#if server.tool_count > 0}
                <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-primary/10 text-primary shrink-0">
                  {server.tool_count} tool{server.tool_count === 1 ? '' : 's'}
                </span>
              {/if}
            </div>
            <div class="flex items-center gap-x-2 shrink-0">
              <!-- Enable/disable toggle -->
              <button
                type="button"
                class="relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200
                  {server.enabled ? 'bg-primary' : 'bg-muted'}"
                role="switch"
                aria-label="{server.enabled ? 'Disable' : 'Enable'} {server.name}"
                aria-checked={server.enabled}
                onclick={() => toggleServer(server.name)}
              >
                <span
                  class="pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow transition duration-200
                    {server.enabled ? 'translate-x-4' : 'translate-x-0'}"
                ></span>
              </button>
              <!-- Delete -->
              {#if !server.builtin}
                <button
                  type="button"
                  class="p-1.5 rounded-md text-muted-foreground-1 hover:text-destructive-foreground hover:bg-muted-hover transition-colors"
                  title="Remove server"
                  onclick={() => removeServer(server.name)}
                >
                  <Trash size={14} />
                </button>
              {/if}
            </div>
          </div>
          {#if server.url}
            <div class="px-4 pb-3">
              <p class="text-xs text-muted-foreground-1 font-mono truncate">{server.url}</p>
            </div>
          {/if}
        </div>
      {/each}
    {/if}
  </div>
</div>
