<script lang="ts">
  import { tabStore } from '../../stores/tabs.svelte.ts';
  import type { TabView } from '../../stores/tabs.svelte.ts';
  import { vmStore } from '../../stores/vms.svelte.ts';
  import { gatewayStore } from '../../stores/gateway.svelte.ts';
  import ArrowClockwise from 'phosphor-svelte/lib/ArrowClockwise';
  import Stop from 'phosphor-svelte/lib/Stop';
  import Trash from 'phosphor-svelte/lib/Trash';
  import GitFork from 'phosphor-svelte/lib/GitFork';
  import FloppyDisk from 'phosphor-svelte/lib/FloppyDisk';
  import List from 'phosphor-svelte/lib/List';
  import Info from 'phosphor-svelte/lib/Info';
  import GearSix from 'phosphor-svelte/lib/GearSix';
  import Terminal from 'phosphor-svelte/lib/Terminal';
  import ChartBar from 'phosphor-svelte/lib/ChartBar';
  import FolderSimple from 'phosphor-svelte/lib/FolderSimple';
  import Scroll from 'phosphor-svelte/lib/Scroll';
  import HardDrives from 'phosphor-svelte/lib/HardDrives';

  let active = $derived(tabStore.active);
  let isVM = $derived(active?.vmId != null);
  let menuOpen = $state(false);
  let busy = $derived(vmStore.loading);

  const vmViewButtons: { view: TabView; label: string; icon: typeof Terminal }[] = [
    { view: 'terminal', label: 'Terminal', icon: Terminal },
    { view: 'stats', label: 'Stats', icon: ChartBar },
    { view: 'files', label: 'Files', icon: FolderSimple },
  ];

  function switchView(view: TabView) {
    if (active) {
      tabStore.updateView(active.id, view);
    }
  }

  function onClickOutside(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (!target.closest('[data-menu]')) {
      menuOpen = false;
    }
  }
</script>

<svelte:document onclick={onClickOutside} />

<div class="flex items-center gap-x-2 bg-layer border-b border-line-2 px-2 py-1">
  <!-- VM actions (only shown when viewing a VM) -->
  {#if isVM}
  <div class="flex items-center gap-x-0.5">
    <button
      type="button"
      class="size-7 inline-flex items-center justify-center rounded-lg text-foreground/70 hover:text-foreground hover:bg-muted-hover disabled:opacity-40 disabled:pointer-events-none"
      disabled={busy}
      aria-label="Restart"
      title="Restart VM"
      onclick={async () => { if (active?.vmId) { await vmStore.stop(active.vmId); } }}
    >
      <ArrowClockwise size={16} />
    </button>
    <button
      type="button"
      class="size-7 inline-flex items-center justify-center rounded-lg text-foreground/70 hover:text-foreground hover:bg-muted-hover disabled:opacity-40 disabled:pointer-events-none"
      disabled={busy}
      aria-label="Stop"
      title="Stop VM"
      onclick={async () => { if (active?.vmId) await vmStore.stop(active.vmId); }}
    >
      <Stop size={16} />
    </button>
    <button
      type="button"
      class="size-7 inline-flex items-center justify-center rounded-lg text-foreground/70 hover:text-foreground hover:bg-muted-hover disabled:opacity-40 disabled:pointer-events-none"
      disabled={busy}
      aria-label="Save"
      title="Save VM (make persistent)"
      onclick={async () => { if (active?.vmId) await vmStore.persist(active.vmId); }}
    >
      <FloppyDisk size={16} />
    </button>
    <button
      type="button"
      class="size-7 inline-flex items-center justify-center rounded-lg text-foreground/70 hover:text-destructive hover:bg-muted-hover disabled:opacity-40 disabled:pointer-events-none"
      disabled={busy}
      aria-label="Destroy"
      title="Destroy VM"
      onclick={async () => { if (active?.vmId) await vmStore.delete(active.vmId); }}
    >
      <Trash size={16} />
    </button>
    <button
      type="button"
      class="size-7 inline-flex items-center justify-center rounded-lg text-foreground/70 hover:text-foreground hover:bg-muted-hover disabled:opacity-40 disabled:pointer-events-none"
      disabled={busy}
      aria-label="Fork"
      title="Fork VM"
      onclick={async () => { if (active?.vmId) await vmStore.fork(active.vmId, { name: `fork-${Date.now()}` }); }}
    >
      <GitFork size={16} />
    </button>
  </div>
  {/if}

  <!-- Spacer -->
  <div class="flex-1"></div>

  <!-- Right: view switcher + menu -->
  {#if isVM}
    <div class="flex items-center bg-background-1 rounded-lg p-0.5">
      {#each vmViewButtons as btn}
        <button
          type="button"
          class="inline-flex items-center gap-x-1 px-2 py-1 text-xs rounded-md transition-colors
            {active?.view === btn.view
              ? 'bg-layer text-foreground shadow-sm'
              : 'text-muted-foreground-1 hover:text-foreground'}"
          onclick={() => switchView(btn.view)}
          title={btn.label}
        >
          <btn.icon size={14} />
          <span class="hidden sm:inline">{btn.label}</span>
        </button>
      {/each}
    </div>
  {/if}

  <div class="relative" data-menu>
    <button
      type="button"
      class="size-7 inline-flex items-center justify-center rounded-lg text-foreground/70 hover:text-foreground hover:bg-muted-hover"
      onclick={(e: MouseEvent) => { e.stopPropagation(); menuOpen = !menuOpen; }}
      aria-label="Menu"
      title="Menu"
    >
      <List size={16} />
    </button>

    {#if menuOpen}
      <div class="absolute end-0 top-full mt-1 w-64 bg-dropdown border border-dropdown-border rounded-xl shadow-lg z-50">
        <div class="p-1">
          {#if isVM}
            <button
              type="button"
              class="w-full flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover"
              onclick={() => { if (active) tabStore.updateView(active.id, 'logs'); menuOpen = false; }}
            >
              <Scroll size={16} />
              <span>VM Logs</span>
            </button>
          {/if}
          <button
            type="button"
            class="w-full flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover"
            onclick={() => { tabStore.openSingleton('logs', 'Service Logs'); menuOpen = false; }}
          >
            <HardDrives size={16} />
            <span>Service Logs</span>
          </button>
          <div class="border-t border-dropdown-border my-1"></div>
          <button
            type="button"
            class="w-full flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover"
            onclick={() => { tabStore.openSingleton('settings', 'Settings'); menuOpen = false; }}
          >
            <GearSix size={16} />
            <span>Settings</span>
          </button>
          <button
            type="button"
            class="w-full flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover"
            onclick={() => { tabStore.openSingleton('settings', 'Settings'); menuOpen = false; }}
          >
            <Info size={16} />
            <span>About Capsem</span>
          </button>

          <!-- Status line -->
          <div class="border-t border-dropdown-border my-1"></div>
          <div class="flex items-center gap-x-2 px-3 py-1.5">
            <span class="size-1.5 rounded-full {gatewayStore.connected ? 'bg-green-500' : gatewayStore.reachable ? 'bg-amber-500' : 'bg-red-500'}"></span>
            <span class="text-xs text-muted-foreground">
              {#if gatewayStore.connected}
                Gateway {gatewayStore.version ?? ''} -- {vmStore.serviceStatus === 'running' ? `${vmStore.vms.length} VM${vmStore.vms.length !== 1 ? 's' : ''}` : vmStore.serviceStatus === 'unavailable' ? 'service down' : 'service unknown'}
              {:else if gatewayStore.reachable}
                Gateway {gatewayStore.version ?? ''} -- needs rebuild
              {:else}
                Offline -- mock mode
              {/if}
            </span>
          </div>
        </div>
      </div>
    {/if}
  </div>
</div>
