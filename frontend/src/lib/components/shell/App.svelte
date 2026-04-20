<script lang="ts">
  import { onMount } from 'svelte';
  import { initTauriLog } from '../../tauri-log';
  import TabBar from './TabBar.svelte';
  import Toolbar from './Toolbar.svelte';
  import NewTabPage from './NewTabPage.svelte';
  import VMFrame from './VMFrame.svelte';

  // Heavy views split into separate chunks; loaded on first use.
  const loadSettings = () => import('./SettingsPage.svelte').then(m => m.default);
  const loadStats = () => import('../views/StatsView.svelte').then(m => m.default);
  const loadLogs = () => import('../views/LogsView.svelte').then(m => m.default);
  const loadServiceLogs = () => import('../views/ServiceLogsView.svelte').then(m => m.default);
  const loadFiles = () => import('../views/FilesView.svelte').then(m => m.default);
  const loadInspector = () => import('../views/InspectorView.svelte').then(m => m.default);
  const loadWizard = () => import('../onboarding/OnboardingWizard.svelte').then(m => m.default);
  const loadCreateDialog = () => import('./CreateSandboxDialog.svelte').then(m => m.default);
  import { tabStore } from '../../stores/tabs.svelte.ts';
  import { gatewayStore } from '../../stores/gateway.svelte.ts';
  import { vmStore } from '../../stores/vms.svelte.ts';
  import { onboardingStore } from '../../stores/onboarding.svelte.ts';
  import { openUrl } from '../../api';

  const vmViews = ['terminal', 'stats', 'logs', 'files', 'inspector'] as const;

  function handleExternalLinkClick(e: MouseEvent) {
    const a = (e.target as Element | null)?.closest('a');
    if (!a) return;
    const href = a.getAttribute('href');
    if (!href) return;
    const external = a.target === '_blank' || /^https?:|^mailto:/.test(href);
    if (!external) return;
    e.preventDefault();
    openUrl(href).catch(err => console.error('[app] openUrl failed:', err));
  }

  async function handleKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'n') {
      e.preventDefault();
      try {
        const { id, name } = await vmStore.provision({ ram_mb: 2048, cpus: 2, persistent: false });
        tabStore.openVM(id, name);
      } catch {
        // Error handled by vmStore.error
      }
    }
  }

  initTauriLog();

  onMount(() => {
    (async () => {
      await gatewayStore.init();
      vmStore.startPolling();

      // Check if onboarding wizard should show
      if (gatewayStore.connected) {
        await onboardingStore.checkOnboarding();
      }

      const params = new URLSearchParams(window.location.search);
      const connectId = params.get('connect');
      const action = params.get('action');
      console.log('[app] init: origin=%s connect=%s action=%s', window.location.origin, connectId, action);

      if (connectId) {
        console.log('[app] deep-link from URL: connect=%s', connectId);
        const vm = vmStore.vms.find(v => v.id === connectId);
        tabStore.openVM(connectId, vm?.name ?? connectId);
      }

      if (connectId || action) {
        history.replaceState(null, '', window.location.pathname);
      }
    })();

    (window as any).__capsemDeepLink = (p: { connect?: string, action?: string }) => {
      console.log('[app] __capsemDeepLink called:', p);
      if (p.connect) {
        const vm = vmStore.vms.find(v => v.id === p.connect);
        console.log('[app] deep-link: opening VM tab connect=%s vm=%s', p.connect, vm?.name ?? 'unknown');
        tabStore.openVM(p.connect, vm?.name ?? p.connect);
      }
      if (p.action && p.connect) {
        // Defer until the tab is mounted and Toolbar is listening.
        requestAnimationFrame(() =>
          window.dispatchEvent(new CustomEvent('capsem:tab-action', {
            detail: { vmId: p.connect, action: p.action },
          })),
        );
      }
    };

    return () => {
      vmStore.destroy();
      gatewayStore.destroy();
      onboardingStore.destroy();
      delete (window as any).__capsemDeepLink;
    };
  });
</script>

<svelte:window onkeydown={handleKeydown} />
<svelte:document onclick={handleExternalLinkClick} />

<div class="flex flex-col h-full">
  <TabBar />
  <Toolbar />

  {#if gatewayStore.connected && !onboardingStore.loading && !onboardingStore.installCompleted}
    <div class="flex items-center gap-3 px-4 py-2 border-b border-line-2 bg-warning/10 text-sm">
      <svg class="size-4 shrink-0 text-warning" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M12 9v4m0 4h.01M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
      <span class="text-foreground flex-1">
        Install didn't finish &mdash; some features may not work.
        {#if onboardingStore.retryError}
          <span class="text-destructive ml-2">{onboardingStore.retryError}</span>
        {/if}
      </span>
      <button
        type="button"
        class="py-1 px-3 text-xs font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
        disabled={onboardingStore.retrying}
        onclick={() => onboardingStore.retryInstall()}
      >
        {onboardingStore.retrying ? 'Retrying...' : 'Retry install'}
      </button>
    </div>
  {/if}

  <div class="flex-1 overflow-hidden bg-background relative">
    {#if !gatewayStore.connected}
      <div class="absolute inset-0 flex flex-col items-center justify-center gap-y-4 bg-background z-10">
        <div class="size-12 rounded-full bg-muted flex items-center justify-center">
          <svg class="size-6 text-muted-foreground" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M12 9v4m0 4h.01M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </div>
        <div class="text-center">
          <p class="text-lg font-medium text-foreground">Capsem service is not running</p>
          <p class="text-sm text-muted-foreground-1 mt-1">Start the service to connect:</p>
          <code class="mt-3 inline-block px-4 py-2 rounded-lg bg-background-1 text-sm font-mono text-foreground">capsem start</code>
        </div>
        <button
          type="button"
          class="mt-2 py-2 px-4 text-sm font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover transition-colors"
          onclick={() => gatewayStore.init()}
        >
          Retry
        </button>
        {#if gatewayStore.error}
          <p class="text-xs text-muted-foreground">{gatewayStore.error}</p>
        {/if}
      </div>
    {:else}
      {#each tabStore.stableTabs as tab (tab.id)}
        {@const isActive = tab.id === tabStore.activeId}
        {@const isVM = tab.vmId != null && vmViews.includes(tab.view as any)}
        <div class="absolute inset-0" hidden={!isActive}>
          {#if tab.view === 'new-tab'}
            <NewTabPage />
          {:else if tab.view === 'settings'}
            {#await loadSettings() then Component}
              <Component />
            {/await}
          {:else if tab.view === 'logs' && !tab.vmId}
            {#await loadServiceLogs() then Component}
              <Component />
            {/await}
          {:else if isVM && tab.vmId}
            <div class="h-full relative">
              <div class="absolute inset-0" class:hidden={tab.view !== 'terminal'}>
                <VMFrame vmId={tab.vmId} tabId={tab.id} />
              </div>
              {#if tab.view === 'stats'}
                <div class="absolute inset-0">
                  {#await loadStats() then Component}<Component vmId={tab.vmId} />{/await}
                </div>
              {:else if tab.view === 'logs'}
                <div class="absolute inset-0">
                  {#await loadLogs() then Component}<Component vmId={tab.vmId} />{/await}
                </div>
              {:else if tab.view === 'files'}
                <div class="absolute inset-0">
                  {#await loadFiles() then Component}<Component vmId={tab.vmId} />{/await}
                </div>
              {:else if tab.view === 'inspector'}
                <div class="absolute inset-0">
                  {#await loadInspector() then Component}<Component vmId={tab.vmId} />{/await}
                </div>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    {/if}
  </div>

  <!-- Onboarding wizard: fixed overlay, renders on top of everything -->
  {#if onboardingStore.needsOnboarding && !onboardingStore.loading}
    {#await loadWizard() then Component}
      <Component />
    {/await}
  {/if}

  {#if vmStore.showCreateModal}
    {#await loadCreateDialog() then Component}
      <Component />
    {/await}
  {/if}
</div>
