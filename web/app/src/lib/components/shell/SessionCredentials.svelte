<script lang="ts">
  import {onMount} from 'svelte';
  import {ConnectionState, GrantState} from '@capsem/sdk';
  import * as api from '../../api';
  import {availableCapabilities, capabilityLabel, SessionGrantFlow, type SessionGrantViewState} from '../../models/credential-flows';

  let {session, onclose}: {session: {id: string; name?: string}; onclose: () => void} = $props();
  let dialog: HTMLDialogElement;
  let flow: SessionGrantFlow;
  let view = $state<SessionGrantViewState>({catalog: null, grants: null, busy: true, error: null});
  let connectionId = $state('');
  let selected = $state<string[]>([]);
  const capabilities = $derived(availableCapabilities(view.catalog, connectionId, view.grants?.denied_capability_ids ?? []));
  const canGrant = $derived(!view.busy && !!view.grants && selected.length > 0 && selected.every(id => capabilities.some(c => c.id === id)));
  onMount(() => {
    flow = new SessionGrantFlow(session.id, api, next => { view = next; });
    dialog.showModal();
    void flow.load();
    return () => flow.dispose();
  });
</script>

<dialog bind:this={dialog} onclose={onclose} class="m-auto w-full max-w-lg max-h-[90vh] overflow-y-auto rounded-xl border border-overlay-border bg-overlay text-foreground shadow-xl backdrop:bg-black/50 p-0" aria-label="Session credentials">
  <div class="flex items-center justify-between gap-3 border-b border-overlay-border px-4 py-3">
    <h2 class="text-lg font-medium">Credentials for {session.name ?? session.id}</h2>
    <button type="button" class="px-3 py-1.5 rounded-md bg-muted hover:bg-muted-hover" onclick={onclose} aria-label="Close session credentials">Close</button>
  </div>
  <div class="p-4">
    <p class="text-sm text-muted-foreground-1">Give this session access to selected capabilities. Account sign-in stays on the host.</p>
    <button type="button" class="mt-3 px-3 py-1.5 text-xs rounded-md bg-muted hover:bg-muted-hover disabled:opacity-60" disabled={view.busy} onclick={() => flow.load()}>Refresh access</button>
    {#if view.error}<p class="mt-3 text-sm text-destructive-text" role="alert">{view.error}</p>{/if}
    {#if view.busy && !view.grants}<p class="mt-3 text-sm text-muted-foreground-1" role="status">Loading session access…</p>{/if}
    {#if view.catalog && view.grants}
      <label class="mt-4 block text-sm">
        Connection
        <select bind:value={connectionId} onchange={() => { selected = []; }} disabled={view.busy} class="mt-1 block w-full border border-line-2 rounded-lg bg-layer px-3 py-2 text-foreground">
          <option value="">Choose a connected account</option>
          {#each view.catalog.connections.filter(c => c.state === ConnectionState.CONNECTED) as connection (connection.id)}
            <option value={connection.id}>{connection.account_label ?? view.catalog.scope_profiles.find(p => p.id === connection.scope_profile_id)?.label ?? 'Google account'}</option>
          {/each}
        </select>
      </label>
      {#if !view.catalog.connections.some(c => c.state === ConnectionState.CONNECTED)}<p class="mt-2 text-xs text-muted-foreground-1">Connect an account in Settings → Plugins first.</p>{/if}
      {#if connectionId && !capabilities.length}<p class="mt-3 text-xs text-muted-foreground-1">No supported capabilities are permitted for this connection and session.</p>{/if}
      <fieldset class="mt-3 space-y-2" disabled={view.busy}>
        <legend class="text-sm font-medium mb-2">Capabilities</legend>
        {#each capabilities as capability (capability.id)}
          <label class="flex items-start gap-2 text-sm">
            <input type="checkbox" bind:group={selected} value={capability.id} class="mt-1 accent-primary" />
            <span>{capability.label}<span class="block text-xs text-muted-foreground-1">{capability.consumer}{capability.required_scopes.length ? ` · ${capability.required_scopes.join(', ')}` : ''}</span></span>
          </label>
        {/each}
      </fieldset>
      <button type="button" disabled={!canGrant} class="mt-3 px-3 py-2 text-sm rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover disabled:opacity-60" onclick={() => flow.grant(connectionId, selected)}>Grant access</button>
      <p class="mt-2 text-xs text-muted-foreground-1">Access expires automatically. Each approval shows its expiry below.</p>
      <h3 class="mt-5 text-sm font-medium">Session approvals</h3>
      {#if !view.grants.grants.length}<p class="mt-2 text-xs text-muted-foreground-1">No access has been granted.</p>{/if}
      <ul class="mt-2 space-y-3">
        {#each view.grants.grants as grant (grant.id)}
          <li class="border-t border-card-line pt-3 text-sm">
            <p>{grant.capability_ids.map(id => capabilityLabel(view.catalog, grant.connection_id, id)).join(', ')}</p>
            <p class="mt-1 text-xs text-muted-foreground-1">{grant.state}{grant.state === GrantState.ACTIVE ? (grant.runtime_bound ? ' · attached to this session' : ' · approval retained') : ''} · expires {new Date(grant.expires_at).toLocaleString()}</p>
            {#if grant.state === GrantState.ACTIVE}<button type="button" class="mt-2 px-3 py-1.5 text-xs rounded-md border border-line-2 text-destructive-text hover:bg-muted-hover disabled:opacity-60" disabled={view.busy} onclick={() => flow.revoke(grant.id)}>Revoke access</button>{/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</dialog>
