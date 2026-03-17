<script lang="ts">
  import { wizardStore } from '../../stores/wizard.svelte';
  import { mcpStore } from '../../stores/mcp.svelte';

  let newName = $state('');
  let newUrl = $state('');
  let newToken = $state('');

  const httpServers = $derived(
    mcpStore.servers.filter((s) => !s.unsupported_stdio),
  );

  async function addServer() {
    if (!newName.trim() || !newUrl.trim()) return;
    await mcpStore.addServer(newName.trim(), newUrl.trim(), {}, newToken.trim() || null);
    newName = '';
    newUrl = '';
    newToken = '';
  }

  async function removeServer(name: string) {
    await mcpStore.removeServer(name);
  }
</script>

<div class="space-y-6">
  <div>
    <h2 class="text-2xl font-semibold">MCP Servers</h2>
    <p class="text-sm text-base-content/60 mt-1">
      Add HTTP MCP servers for AI agent tool access. This step is entirely optional.
    </p>
  </div>

  <!-- Existing servers -->
  {#if httpServers.length > 0}
    <div class="space-y-2">
      {#each httpServers as server}
        <div class="flex items-center justify-between border border-base-300 rounded-lg p-3">
          <div>
            <span class="font-medium text-sm">{server.name}</span>
            <span class="text-xs text-base-content/40 ml-2">{server.url}</span>
            {#if server.has_bearer_token}
              <span class="badge badge-sm ml-2 text-xs text-base-content/40">auth</span>
            {/if}
          </div>
          <button
            class="btn btn-ghost btn-xs text-denied"
            onclick={() => removeServer(server.name)}
          >
            Remove
          </button>
        </div>
      {/each}
    </div>
  {:else}
    <p class="text-sm text-base-content/40">No HTTP MCP servers configured.</p>
  {/if}

  <!-- Add server form -->
  <div class="card border border-base-300 p-4 space-y-3">
    <h3 class="text-sm font-semibold">Add Server</h3>
    <div class="grid grid-cols-2 gap-3">
      <div>
        <label class="text-xs text-base-content/50" for="mcp-name">Name</label>
        <input
          id="mcp-name"
          type="text"
          class="input input-sm input-bordered w-full text-xs"
          placeholder="my-server"
          bind:value={newName}
        />
      </div>
      <div>
        <label class="text-xs text-base-content/50" for="mcp-url">URL</label>
        <input
          id="mcp-url"
          type="url"
          class="input input-sm input-bordered w-full text-xs"
          placeholder="https://mcp.example.com/v1"
          bind:value={newUrl}
        />
      </div>
    </div>
    <div>
      <label class="text-xs text-base-content/50" for="mcp-token">Bearer token (optional)</label>
      <input
        id="mcp-token"
        type="password"
        class="input input-sm input-bordered w-full text-xs"
        placeholder="Optional"
        bind:value={newToken}
      />
    </div>
    <button
      class="btn bg-interactive text-white btn-sm w-fit"
      disabled={!newName.trim() || !newUrl.trim()}
      onclick={addServer}
    >
      Add
    </button>
  </div>

  <p class="text-xs text-base-content/40">
    Policy and tool permissions can be configured later in Settings.
  </p>

  <!-- Nav -->
  <div class="flex justify-between pt-4">
    <button class="btn btn-ghost btn-sm" onclick={() => wizardStore.back()}>Back</button>
    <div class="flex gap-2">
      <button class="btn btn-ghost btn-sm" onclick={() => wizardStore.next()}>Skip</button>
      <button class="btn bg-interactive text-white btn-sm" onclick={() => wizardStore.next()}>
        Next
      </button>
    </div>
  </div>
</div>
