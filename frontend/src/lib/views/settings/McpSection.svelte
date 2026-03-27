<script lang="ts">
  import { mcpStore } from '../../stores/mcp.svelte';
  import type { McpServerInfo, McpToolInfo, McpPolicyInfo } from '../../types';

  // Add-server form state.
  let showAddForm = $state(false);
  let newName = $state('');
  let newUrl = $state('');
  let newBearerToken = $state('');
  let newHeaderPairs = $state<{ key: string; value: string }[]>([]);

  // Server expand/collapse state.
  let expandedServers = $state<Set<string>>(new Set());
  let expandedToolGroups = $state<Set<string>>(new Set());
  let expandedAuth = $state<Set<string>>(new Set());

  // Refreshing state.
  let refreshing = $state(false);

  /** Builtin tools (server_name === 'builtin'), grouped by category. */
  const builtinTools = $derived(mcpStore.tools.filter((t) => t.server_name === 'builtin'));
  const builtinGroups = $derived(() => {
    const groups: { name: string; tools: typeof builtinTools }[] = [];
    const http = builtinTools.filter((t) => !t.original_name.startsWith('snapshots_'));
    const snapshots = builtinTools.filter((t) => t.original_name.startsWith('snapshots_'));
    if (http.length > 0) groups.push({ name: 'HTTP', tools: http });
    if (snapshots.length > 0) groups.push({ name: 'Snapshots', tools: snapshots });
    return groups;
  });

  /** HTTP servers (not unsupported_stdio). */
  const httpServers = $derived(mcpStore.servers.filter((s) => !s.unsupported_stdio));

  /** Unsupported stdio servers (from auto-detection). */
  const stdioServers = $derived(mcpStore.servers.filter((s) => s.unsupported_stdio));

  function resetAddForm() {
    newName = '';
    newUrl = '';
    newBearerToken = '';
    newHeaderPairs = [];
    showAddForm = false;
  }

  async function handleAddServer() {
    const name = newName.trim();
    const url = newUrl.trim();
    if (!name || !url) return;
    const headers: Record<string, string> = {};
    for (const pair of newHeaderPairs) {
      const k = pair.key.trim();
      if (k) headers[k] = pair.value;
    }
    const token = newBearerToken.trim() || null;
    await mcpStore.addServer(name, url, headers, token);
    resetAddForm();
  }

  function addHeaderPair() {
    newHeaderPairs = [...newHeaderPairs, { key: '', value: '' }];
  }

  function removeHeaderPair(index: number) {
    newHeaderPairs = newHeaderPairs.filter((_, i) => i !== index);
  }

  function toggleToolGroup(name: string) {
    const next = new Set(expandedToolGroups);
    if (next.has(name)) next.delete(name); else next.add(name);
    expandedToolGroups = next;
  }

  function toggleServer(name: string) {
    const next = new Set(expandedServers);
    if (next.has(name)) next.delete(name); else next.add(name);
    expandedServers = next;
  }

  function toggleAuth(name: string) {
    const next = new Set(expandedAuth);
    if (next.has(name)) next.delete(name); else next.add(name);
    expandedAuth = next;
  }

  async function handleRefresh() {
    refreshing = true;
    try {
      await mcpStore.refresh();
    } finally {
      refreshing = false;
    }
  }

  /** Get the effective permission for a tool from policy. */
  function toolPermission(policy: McpPolicyInfo, tool: McpToolInfo): string {
    return policy.tool_permissions[tool.namespaced_name] ?? policy.default_tool_permission;
  }
</script>

<!-- MCP Servers section -->
<div class="mb-4">
  <h1 class="text-2xl font-bold">MCP Servers</h1>
  <p class="text-sm text-base-content/50">
    Configure Model Context Protocol servers and tool permissions.
  </p>
</div>

{#if mcpStore.loading}
  <div class="flex items-center justify-center py-8">
    <span class="loading loading-spinner loading-md"></span>
  </div>
{:else if mcpStore.error}
  <div class="text-sm text-denied">{mcpStore.error}</div>
{:else}

<!-- Policy subsection -->
<div data-subgroup="mcp-policy" class="scroll-mt-4">
  <h2 class="text-lg font-semibold text-interactive mb-3">Policy</h2>

  <div class="space-y-3">
    <div class="flex items-start gap-3">
      <div class="flex-1 min-w-0">
        <span class="text-sm font-medium">Default Tool Permission</span>
        <p class="text-xs text-base-content/50">Permission applied to tools without an explicit override.</p>
      </div>
      <select
        class="select select-sm select-bordered w-28 text-xs"
        value={mcpStore.policy.default_tool_permission}
        onchange={(e) => mcpStore.setDefaultPermission(e.currentTarget.value)}
      >
        <option value="allow">allow</option>
        <option value="warn">warn</option>
        <option value="block">block</option>
      </select>
    </div>

    {#if mcpStore.policy.blocked_servers.length > 0}
      <div class="border-t border-base-200/50 pt-3">
        <span class="text-sm font-medium">Blocked Servers</span>
        <p class="text-xs text-base-content/50 mb-1.5">Servers blocked by corp policy (read-only).</p>
        <div class="flex flex-wrap gap-1.5">
          {#each mcpStore.policy.blocked_servers as name}
            <span class="badge badge-sm bg-denied/15 text-denied font-mono">{name}</span>
          {/each}
        </div>
      </div>
    {/if}

  </div>
</div>

<!-- Local tools subsection -->
<div data-subgroup="mcp-local" class="mt-8 scroll-mt-4">
  <h2 class="text-lg font-semibold text-interactive mb-3">Local Tools</h2>
  <p class="text-xs text-base-content/50 mb-3">Built-in tools that run on the host, governed by network policy.</p>

  {#if builtinTools.length === 0}
    <p class="text-sm text-base-content/40">No built-in tools available.</p>
  {:else}
    {#each builtinGroups() as group (group.name)}
      {@const isOpen = expandedToolGroups.has(group.name)}
      <button
        class="flex items-center gap-2 mt-3 mb-1 w-full text-left"
        onclick={() => toggleToolGroup(group.name)}
      >
        <svg class="size-3 text-base-content/40 transition-transform {isOpen ? 'rotate-90' : ''}" viewBox="0 0 24 24" fill="currentColor"><path d="M8 5v14l11-7z"/></svg>
        <span class="text-sm font-medium text-base-content/60">{group.name}</span>
        <span class="text-xs text-base-content/30">{group.tools.length} tools</span>
      </button>
      {#if isOpen}
        <div class="overflow-x-auto">
          <table class="table table-xs">
            <thead>
              <tr class="text-base-content/50">
                <th>Tool</th>
                <th class="text-center">Read-only</th>
                <th class="text-center">Destructive</th>
                <th class="text-center">Idempotent</th>
                <th class="text-center">Open World</th>
                <th class="text-right">Permission</th>
              </tr>
            </thead>
            <tbody>
              {#each group.tools as tool (tool.namespaced_name)}
                {@const perm = toolPermission(mcpStore.policy, tool)}
                <tr>
                  <td class="font-mono text-xs">{tool.original_name}</td>
                  <td class="text-center">{#if tool.annotations?.read_only_hint}<span class="text-allowed">Y</span>{:else}<span class="text-base-content/20">--</span>{/if}</td>
                  <td class="text-center">{#if tool.annotations?.destructive_hint}<span class="text-denied">Y</span>{:else}<span class="text-base-content/20">--</span>{/if}</td>
                  <td class="text-center">{#if tool.annotations?.idempotent_hint}<span class="text-base-content/60">Y</span>{:else}<span class="text-base-content/20">--</span>{/if}</td>
                  <td class="text-center">{#if tool.annotations?.open_world_hint}<span class="text-base-content/60">Y</span>{:else}<span class="text-base-content/20">--</span>{/if}</td>
                  <td class="text-right">
                    <select
                      class="select select-xs select-bordered w-20 text-xs"
                      value={perm}
                      onchange={(e) => mcpStore.setToolPermission(tool.namespaced_name, e.currentTarget.value)}
                    >
                      <option value="allow">allow</option>
                      <option value="warn">warn</option>
                      <option value="block">block</option>
                    </select>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    {/each}
  {/if}
</div>

<!-- Servers subsection -->
<div data-subgroup="mcp-servers" class="mt-8 scroll-mt-4">
  <div class="flex items-center gap-3 mb-3">
    <h2 class="text-lg font-semibold text-interactive">Servers</h2>
    <button
      class="btn btn-ghost btn-xs text-xs text-base-content/50"
      disabled={refreshing}
      onclick={handleRefresh}
    >
      {#if refreshing}
        <span class="loading loading-spinner loading-xs"></span>
      {/if}
      Refresh Tools
    </button>
  </div>

  <!-- Add server form -->
  <div class="mb-3">
    {#if showAddForm}
      <div class="card card-bordered overflow-hidden">
        <div class="px-4 py-3 bg-base-200/40 space-y-2">
          <div class="flex items-center gap-2">
            <span class="text-sm font-medium">Add MCP Server</span>
            <button
              class="btn btn-ghost btn-xs ml-auto"
              onclick={resetAddForm}
            >Cancel</button>
          </div>
          <div class="grid grid-cols-2 gap-2">
            <div class="form-control">
              <label for="mcp-add-name" class="text-xs text-base-content/50 mb-0.5">Name</label>
              <input
                id="mcp-add-name"
                type="text"
                class="input input-sm input-bordered font-mono text-xs"
                placeholder="my-server"
                bind:value={newName}
              />
            </div>
            <div class="form-control">
              <label for="mcp-add-url" class="text-xs text-base-content/50 mb-0.5">URL</label>
              <input
                id="mcp-add-url"
                type="url"
                class="input input-sm input-bordered font-mono text-xs"
                placeholder="https://mcp.example.com/v1"
                bind:value={newUrl}
              />
            </div>
          </div>
          <div class="form-control">
            <label for="mcp-add-bearer" class="text-xs text-base-content/50 mb-0.5">Bearer Token (optional)</label>
            <input
              id="mcp-add-bearer"
              type="password"
              class="input input-sm input-bordered font-mono text-xs"
              placeholder="tok_..."
              bind:value={newBearerToken}
            />
          </div>
          {#if newHeaderPairs.length > 0}
            <div class="space-y-1">
              <span class="text-xs text-base-content/50">Custom Headers</span>
              {#each newHeaderPairs as pair, i}
                <div class="flex items-center gap-1">
                  <input
                    type="text"
                    class="input input-xs input-bordered font-mono text-xs flex-1"
                    placeholder="Header-Name"
                    bind:value={pair.key}
                  />
                  <span class="text-xs text-base-content/30">:</span>
                  <input
                    type="text"
                    class="input input-xs input-bordered font-mono text-xs flex-1"
                    placeholder="value"
                    bind:value={pair.value}
                  />
                  <button
                    class="btn btn-ghost btn-xs text-base-content/40"
                    title="Remove header"
                    onclick={() => removeHeaderPair(i)}
                  >
                    <svg class="size-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                    </svg>
                  </button>
                </div>
              {/each}
            </div>
          {/if}
          <div class="flex items-center gap-2 pt-1">
            <button class="btn btn-ghost btn-xs text-xs" onclick={addHeaderPair}>
              + Add header
            </button>
            <div class="flex-1"></div>
            <button
              class="btn bg-interactive text-white btn-sm"
              disabled={!newName.trim() || !newUrl.trim()}
              onclick={handleAddServer}
            >Add Server</button>
          </div>
        </div>
      </div>
    {:else}
      <button
        class="btn btn-ghost btn-sm text-xs text-base-content/50"
        onclick={() => showAddForm = true}
      >+ Add server</button>
    {/if}
  </div>

  <!-- HTTP server cards -->
  <div class="space-y-3">
    {#each httpServers as server (server.name)}
      {@const serverTools = mcpStore.toolsByServer[server.name] ?? []}
      {@const isExpanded = expandedServers.has(server.name)}
      {@const isAuthExpanded = expandedAuth.has(server.name)}
      <div class="card card-bordered overflow-hidden">
        <!-- Server header -->
        <div class="flex items-center gap-3 px-4 py-2.5 bg-base-200/40">
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="flex items-center gap-3 flex-1 min-w-0 cursor-pointer" onclick={() => toggleServer(server.name)}>
            <svg class="size-3 text-base-content/40 transition-transform flex-shrink-0 {isExpanded ? 'rotate-90' : ''}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <polyline points="9,6 15,12 9,18" />
            </svg>
            <span class="font-semibold text-sm">{server.name}</span>
          </div>
          <div class="flex items-center gap-2 flex-shrink-0">
            <span class="text-xs text-base-content/40">
              {server.tool_count} tool{server.tool_count !== 1 ? 's' : ''}
            </span>
            <input
              type="checkbox"
              class="toggle toggle-sm"
              checked={server.enabled}
              onchange={() => mcpStore.toggleServer(server.name, !server.enabled)}
            />
            {#if server.source === 'manual'}
              <button
                class="btn btn-ghost btn-xs text-base-content/40 hover:text-denied"
                title="Remove server"
                onclick={() => mcpStore.removeServer(server.name)}
              >
                <svg class="size-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                </svg>
              </button>
            {/if}
          </div>
        </div>

        <!-- Expanded content -->
        {#if isExpanded}
          <!-- URL -->
          <div class="px-4 py-1.5 border-t border-base-200/50">
            <span class="text-xs text-base-content/40">URL:</span>
            <code class="text-xs font-mono text-base-content/60 ml-1">{server.url}</code>
          </div>

          <!-- Auth section (collapsible) -->
          <div class="border-t border-base-200/50">
            <button
              class="flex items-center gap-2 px-4 py-1.5 text-xs text-base-content/50 hover:text-base-content/70 w-full text-left"
              onclick={() => toggleAuth(server.name)}
            >
              <svg class="size-2.5 transition-transform {isAuthExpanded ? 'rotate-90' : ''}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <polyline points="9,6 15,12 9,18" />
              </svg>
              Auth
              {#if server.has_bearer_token || server.custom_header_count > 0}
                <span class="text-base-content/30">({server.has_bearer_token ? 'Bearer' : ''}{server.has_bearer_token && server.custom_header_count > 0 ? ', ' : ''}{server.custom_header_count > 0 ? `${server.custom_header_count} header${server.custom_header_count !== 1 ? 's' : ''}` : ''})</span>
              {/if}
            </button>
            {#if isAuthExpanded}
              <div class="px-6 pb-2 space-y-1">
                {#if server.has_bearer_token}
                  <div class="text-xs">
                    <span class="text-base-content/40">Bearer:</span>
                    <span class="font-mono text-base-content/50 ml-1">****</span>
                  </div>
                {:else}
                  <div class="text-xs text-base-content/30">No bearer token configured.</div>
                {/if}
                {#if server.custom_header_count > 0}
                  <div class="text-xs text-base-content/40">
                    {server.custom_header_count} custom header{server.custom_header_count !== 1 ? 's' : ''} configured.
                  </div>
                {/if}
              </div>
            {/if}
          </div>

          <!-- Tools table -->
          {#if server.enabled && serverTools.length > 0}
            <div class="border-t border-base-200/50 overflow-x-auto">
              <table class="table table-xs">
                <thead>
                  <tr class="text-base-content/50">
                    <th>Tool</th>
                    <th class="text-center">R/O</th>
                    <th class="text-center">Destr</th>
                    <th class="text-center">Idemp</th>
                    <th class="text-center">OW</th>
                    <th class="text-center">Status</th>
                    <th class="text-right">Permission</th>
                  </tr>
                </thead>
                <tbody>
                  {#each serverTools as tool (tool.namespaced_name)}
                    {@const perm = toolPermission(mcpStore.policy, tool)}
                    <tr>
                      <td class="font-mono text-xs">{tool.original_name}</td>
                      <td class="text-center">{#if tool.annotations?.read_only_hint}<span class="text-allowed">Y</span>{:else}<span class="text-base-content/20">--</span>{/if}</td>
                      <td class="text-center">{#if tool.annotations?.destructive_hint}<span class="text-denied">Y</span>{:else}<span class="text-base-content/20">--</span>{/if}</td>
                      <td class="text-center">{#if tool.annotations?.idempotent_hint}<span class="text-base-content/60">Y</span>{:else}<span class="text-base-content/20">--</span>{/if}</td>
                      <td class="text-center">{#if tool.annotations?.open_world_hint}<span class="text-base-content/60">Y</span>{:else}<span class="text-base-content/20">--</span>{/if}</td>
                      <td class="text-center">
                        {#if tool.pin_changed}
                          <span class="text-xs text-caution">definition changed</span>
                        {:else if tool.approved}
                          <span class="text-xs text-allowed">verified</span>
                        {:else}
                          <button
                            class="btn btn-ghost btn-xs text-xs"
                            onclick={() => mcpStore.approveTool(tool.namespaced_name)}
                          >verify</button>
                        {/if}
                      </td>
                      <td class="text-right">
                        <select
                          class="select select-xs select-bordered w-20 text-xs"
                          value={perm}
                          onchange={(e) => mcpStore.setToolPermission(tool.namespaced_name, e.currentTarget.value)}
                        >
                          <option value="allow">allow</option>
                          <option value="warn">warn</option>
                          <option value="block">block</option>
                        </select>
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {/if}
        {/if}
      </div>
    {/each}
  </div>

  <!-- Unsupported stdio servers -->
  {#if stdioServers.length > 0}
    <div class="mt-4 space-y-2">
      <h3 class="text-sm font-medium text-base-content/40">Unsupported Stdio Servers</h3>
      {#each stdioServers as server (server.name)}
        <div class="card card-bordered overflow-hidden opacity-60">
          <div class="flex items-center gap-3 px-4 py-2 bg-base-200/20">
            <span class="font-semibold text-sm text-base-content/50">{server.name}</span>
            <span class="text-xs text-base-content/30 ml-auto font-mono truncate max-w-xs">{server.url}</span>
          </div>
          <div class="px-4 py-1.5 border-t border-base-200/30 bg-caution/5">
            <p class="text-xs text-caution">
              Stdio server -- must be installed inside the VM. Capsem only proxies HTTP MCP servers.
            </p>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

{/if}
