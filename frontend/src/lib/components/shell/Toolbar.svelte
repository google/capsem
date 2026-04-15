<script lang="ts">
  import { tabStore } from '../../stores/tabs.svelte.ts';
  import type { TabView } from '../../stores/tabs.svelte.ts';
  import { vmStore } from '../../stores/vms.svelte.ts';
  import { gatewayStore } from '../../stores/gateway.svelte.ts';
  import Modal from './Modal.svelte';
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
  import { formatTokens, formatCost } from '../../format';

  let active = $derived(tabStore.active);
  let isVM = $derived(active?.vmId != null);
  let menuOpen = $state(false);
  let busy = $derived(vmStore.loading);
  let activeVm = $derived(isVM && active?.vmId ? vmStore.vms.find(v => v.id === active!.vmId) : null);
  let isPersistent = $derived(activeVm?.persistent ?? false);

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

  // --- Modal state ---
  type ModalKind = 'stop' | 'destroy' | 'save' | 'fork' | null;
  let modalKind = $state<ModalKind>(null);
  let modalInput = $state('');

  function openModal(kind: ModalKind) {
    menuOpen = false;
    if (kind === 'save') {
      modalInput = active?.title ?? '';
    } else if (kind === 'fork') {
      modalInput = `${active?.title ?? 'vm'}-fork`;
    }
    modalKind = kind;
  }

  function closeModal() {
    modalKind = null;
    modalInput = '';
  }

  async function handleModalConfirm() {
    if (!active?.vmId) return;
    const id = active.vmId;
    const kind = modalKind;
    closeModal();
    switch (kind) {
      case 'stop':
        await vmStore.stop(id);
        break;
      case 'destroy':
        await vmStore.delete(id);
        break;
      case 'save':
        if (modalInput.trim()) await vmStore.persist(id, modalInput.trim());
        break;
      case 'fork': {
        if (!modalInput.trim()) break;
        const result = await vmStore.fork(id, { name: modalInput.trim() });
        const forked = vmStore.vms.find(v => v.name === result.name);
        if (forked) tabStore.openVM(forked.id, forked.name ?? result.name);
        break;
      }
    }
  }
</script>

<div class="flex items-center gap-x-2 bg-layer border-b border-line-2 px-2 py-1">
  <!-- Left: menu + view switcher -->
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
      <div class="fixed inset-0 z-40" onclick={() => menuOpen = false}></div>
      <div class="absolute start-0 top-full mt-1 w-64 bg-dropdown border border-dropdown-border rounded-xl shadow-lg z-50">
        <div class="p-1">
          <!-- VM section -->
          {#if isVM}
            <button
              type="button"
              class="w-full flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover"
              onclick={() => { if (active) tabStore.updateView(active.id, 'logs'); menuOpen = false; }}
            >
              <Scroll size={16} />
              <span>VM Logs</span>
            </button>
            {#if isPersistent}
              <button
                type="button"
                class="w-full flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover disabled:opacity-40 disabled:pointer-events-none"
                disabled={busy}
                onclick={async () => { if (active?.vmId) { await vmStore.restart(active.vmId); } menuOpen = false; }}
              >
                <ArrowClockwise size={16} />
                <span>Restart</span>
              </button>
            {/if}
            {#if isPersistent}
              <button
                type="button"
                class="w-full flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover disabled:opacity-40 disabled:pointer-events-none"
                disabled={busy}
                onclick={() => openModal('stop')}
              >
                <Stop size={16} />
                <span>Stop</span>
              </button>
            {/if}
            {#if !isPersistent}
              <button
                type="button"
                class="w-full flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover disabled:opacity-40 disabled:pointer-events-none"
                disabled={busy}
                onclick={() => openModal('save')}
              >
                <FloppyDisk size={16} />
                <span>Save</span>
              </button>
            {/if}
            <button
              type="button"
              class="w-full flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover disabled:opacity-40 disabled:pointer-events-none"
              disabled={busy}
              onclick={() => openModal('fork')}
            >
              <GitFork size={16} />
              <span>Fork</span>
            </button>
            <button
              type="button"
              class="w-full flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover disabled:opacity-40 disabled:pointer-events-none"
              disabled={busy}
              onclick={() => openModal('destroy')}
            >
              <Trash size={16} />
              <span>Destroy</span>
            </button>
            <div class="border-t border-dropdown-border my-1"></div>
          {/if}

          <!-- Service section -->
          <button
            type="button"
            class="w-full flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover"
            onclick={() => { tabStore.openSingleton('logs', 'Service Logs'); menuOpen = false; }}
          >
            <HardDrives size={16} />
            <span>Service Logs</span>
          </button>
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

  <!-- Left: view switcher -->
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

  <!-- Center: window name / shell title -->
  <div class="flex-1 min-w-0 text-center">
    {#if isVM && active?.subtitle}
      <span class="text-xs text-muted-foreground-1 truncate inline-block max-w-full">{active.subtitle}</span>
    {/if}
  </div>

  <!-- Right: stats -->
  {#if isVM && activeVm}
    <div class="flex items-center gap-x-3 text-[11px] text-muted-foreground-1 tabular-nums">
      <span title="Tokens">{formatTokens((activeVm.total_input_tokens ?? 0) + (activeVm.total_output_tokens ?? 0))} tok</span>
      <span title="Tool calls">{activeVm.total_tool_calls ?? 0} calls</span>
      <span title="Cost">{formatCost(activeVm.total_estimated_cost ?? 0)}</span>
    </div>
  {/if}
</div>

<!-- Modals -->
<Modal
  open={modalKind === 'stop'}
  title="Stop Session"
  confirmLabel="Stop"
  destructive
  onconfirm={handleModalConfirm}
  oncancel={closeModal}
>
  <p class="text-sm text-foreground">Stop <strong>{active?.title}</strong>?</p>
  {#if !isPersistent}
    <p class="text-xs text-muted-foreground-1 mt-2">This is an ephemeral session. It will be destroyed.</p>
  {/if}
</Modal>

<Modal
  open={modalKind === 'destroy'}
  title="Destroy Session"
  confirmLabel="Destroy"
  destructive
  onconfirm={handleModalConfirm}
  oncancel={closeModal}
>
  <p class="text-sm text-foreground">Destroy <strong>{active?.title}</strong>? This cannot be undone.</p>
</Modal>

<Modal
  open={modalKind === 'save'}
  title="Save Session"
  confirmLabel="Save"
  onconfirm={handleModalConfirm}
  oncancel={closeModal}
>
  <label for="save-name" class="text-xs font-medium text-foreground block mb-1">Name</label>
  <input
    id="save-name"
    type="text"
    class="w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
    bind:value={modalInput}
  />
</Modal>

<Modal
  open={modalKind === 'fork'}
  title="Fork Session"
  confirmLabel="Fork"
  onconfirm={handleModalConfirm}
  oncancel={closeModal}
>
  <label for="fork-name" class="text-xs font-medium text-foreground block mb-1">Name</label>
  <input
    id="fork-name"
    type="text"
    class="w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
    bind:value={modalInput}
  />
</Modal>
