<script lang="ts">
  import { onMount } from 'svelte';
  import { slide } from 'svelte/transition';
  import { settingsStore } from '../../stores/settings.svelte.ts';
  import { mcpStore } from '../../stores/mcp.svelte.ts';
  import type { McpServerInfo, McpToolInfo } from '../../types';
  import * as api from '../../api';
  import ArrowClockwise from 'phosphor-svelte/lib/ArrowClockwise';
  import CaretDown from 'phosphor-svelte/lib/CaretDown';
  import Plus from 'phosphor-svelte/lib/Plus';
  import Trash from 'phosphor-svelte/lib/Trash';
  import WarningCircle from 'phosphor-svelte/lib/WarningCircle';
  import X from 'phosphor-svelte/lib/X';

  // MCP servers from the settings tree (loaded by SettingsPage onMount)
  let servers = $derived(settingsStore.model?.mcpServers ?? []);
  let userServers = $derived(servers.filter(s => !s.builtin));
  let builtinServers = $derived(servers.filter(s => s.builtin));

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

  // --- Per-tool permission ---
  function toolPermission(toolName: string): string {
    return mcpStore.policy.tool_permissions[toolName] ?? defaultPermission;
  }

  async function handleToolPermission(toolName: string, e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    await mcpStore.setToolPermission(toolName, value);
  }

  // --- Add server form ---
  let showAddForm = $state(false);
  let newName = $state('');
  let newUrl = $state('');
  let newBearerToken = $state('');
  let newHeaders = $state<{ key: string; value: string }[]>([]);
  let saving = $state(false);

  let canAdd = $derived(newName.trim().length > 0 && newUrl.trim().length > 0);

  onMount(() => {
    mcpStore.load();
  });

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

  async function addServer() {
    if (!canAdd) return;
    saving = true;
    try {
      const headers: Record<string, string> = {};
      for (const h of newHeaders) {
        if (h.key.trim()) headers[h.key.trim()] = h.value;
      }
      await api.addMcpServer(
        newName.trim(),
        newUrl.trim(),
        headers,
        newBearerToken.trim() || null,
      );
      await api.reloadConfig();
      resetForm();
      await settingsStore.load();
      await mcpStore.load();
    } finally {
      saving = false;
    }
  }

  async function removeServer(name: string) {
    saving = true;
    try {
      await api.removeMcpServer(name);
      await api.reloadConfig();
      await settingsStore.load();
      await mcpStore.load();
    } finally {
      saving = false;
    }
  }

  async function toggleServer(name: string, currentlyEnabled: boolean) {
    saving = true;
    try {
      await api.setMcpServerEnabled(name, !currentlyEnabled);
      await api.reloadConfig();
      await settingsStore.load();
      await mcpStore.load();
    } finally {
      saving = false;
    }
  }

  async function handlePolicyChange(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    await api.setMcpDefaultPermission(value);
    await api.reloadConfig();
    await settingsStore.load();
    await mcpStore.load();
  }

  // Policy from settings tree
  let defaultPermission = $derived.by(() => {
    const leaf = settingsStore.findLeaf('mcp.policy.default_tool_permission');
    return (leaf?.effective_value as string) ?? 'allow';
  });
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
        </div>
        <div class="shrink-0 mt-0.5">
          <select
            class="py-1 px-2 text-xs rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
            value={toolPermission(tool.namespaced_name)}
            onchange={(e) => handleToolPermission(tool.namespaced_name, e)}
          >
            <option value="allow">Allow</option>
            <option value="ask">Ask</option>
            <option value="block">Block</option>
          </select>
        </div>
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

  <!-- Policy -->
  <div>
    <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">Policy</h3>
    <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
      <div class="flex items-center justify-between p-4">
        <div>
          <p class="text-sm font-medium text-foreground">Default tool permission</p>
          <p class="text-xs text-muted-foreground-1 mt-0.5">Action when an AI agent calls a tool not in any policy</p>
        </div>
        <select
          class="py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary w-32"
          value={defaultPermission}
          onchange={handlePolicyChange}
        >
          <option value="allow">Allow</option>
          <option value="warn">Warn</option>
          <option value="block">Block</option>
        </select>
      </div>
    </div>
  </div>

  <!-- Built-in Servers -->
  {#if builtinServers.length > 0}
    <div>
      <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">Built-in</h3>
      {#each builtinServers as server (server.key)}
        {@const runtime = runtimeByName.get(server.key)}
        {@const tools = mcpStore.toolsByServer[server.key] ?? []}
        {@const isExpanded = expandedGroups.has(server.key)}
        <div class="bg-card border border-card-line rounded-xl mb-3 overflow-hidden">
          <div class="flex items-center justify-between px-4 py-3">
            <button
              type="button"
              class="flex items-center gap-x-3 min-w-0 flex-1 text-left"
              onclick={() => toggleGroup(server.key)}
            >
              <span class="text-sm font-semibold text-foreground font-mono truncate">{server.name}</span>
              <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1 shrink-0">{server.transport}</span>
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
            <div class="flex items-center gap-x-2 shrink-0 ml-2">
              <button
                type="button"
                class="relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200
                  {server.enabled ? 'bg-primary' : 'bg-muted'}
                  {server.corp_locked ? 'opacity-50 cursor-not-allowed' : ''}"
                role="switch"
                aria-label="{server.enabled ? 'Disable' : 'Enable'} {server.name}"
                aria-checked={server.enabled}
                disabled={server.corp_locked || saving}
                onclick={() => toggleServer(server.key, server.enabled)}
              >
                <span
                  class="pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow transition duration-200
                    {server.enabled ? 'translate-x-4' : 'translate-x-0'}"
                ></span>
              </button>
            </div>
          </div>
          {#if server.description && !isExpanded}
            <div class="px-4 pb-3">
              <p class="text-xs text-muted-foreground-1">{server.description}</p>
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
    <div class="flex items-center justify-between mb-2">
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
              disabled={!canAdd || saving}
              onclick={addServer}
            >
              Add Server
            </button>
          </div>
        </div>
      </div>
    {/if}

    <!-- Server list -->
    {#if userServers.length === 0 && !showAddForm}
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
      {#each userServers as server (server.key)}
        {@const runtime = runtimeByName.get(server.key)}
        {@const tools = mcpStore.toolsByServer[server.key] ?? []}
        {@const isExpanded = expandedGroups.has(server.key)}
        <div class="bg-card border border-card-line rounded-xl mb-3 overflow-hidden">
          <div class="flex items-center justify-between px-4 py-3">
            <button
              type="button"
              class="flex items-center gap-x-3 min-w-0 flex-1 text-left"
              onclick={() => toggleGroup(server.key)}
            >
              <span class="text-sm font-semibold text-foreground font-mono truncate">{server.name}</span>
              <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1 shrink-0">{server.transport}</span>
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
            <div class="flex items-center gap-x-2 shrink-0 ml-2">
              <button
                type="button"
                class="relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200
                  {server.enabled ? 'bg-primary' : 'bg-muted'}
                  {server.corp_locked ? 'opacity-50 cursor-not-allowed' : ''}"
                role="switch"
                aria-label="{server.enabled ? 'Disable' : 'Enable'} {server.name}"
                aria-checked={server.enabled}
                disabled={server.corp_locked || saving}
                onclick={() => toggleServer(server.key, server.enabled)}
              >
                <span
                  class="pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow transition duration-200
                    {server.enabled ? 'translate-x-4' : 'translate-x-0'}"
                ></span>
              </button>
              {#if !server.corp_locked}
                <button
                  type="button"
                  class="p-1.5 rounded-md text-muted-foreground-1 hover:text-destructive-foreground hover:bg-muted-hover transition-colors"
                  title="Remove server"
                  disabled={saving}
                  onclick={() => removeServer(server.key)}
                >
                  <Trash size={14} />
                </button>
              {/if}
            </div>
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
