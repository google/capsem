<script lang="ts">
  import {onMount} from 'svelte';
  import {ConnectionState, ProviderRevocationState, TransactionState} from '@capsem/sdk';
  import * as api from '../../api';
  import {ConnectionFlow, type ConnectionViewState} from '../../models/credential-flows';

  let view = $state<ConnectionViewState>({catalog: null, transaction: null, url: null, busy: false, error: null});
  const flow = new ConnectionFlow(api, next => { view = next; });
  const pending = $derived(view.transaction?.state === TransactionState.PENDING);
  onMount(() => { void flow.load(); return () => flow.dispose(); });

  async function openSignIn() {
    if (!view.url) return;
    try { await api.openUrl(view.url); }
    catch { view = {...view, error: 'Could not open the browser. Try opening sign-in again.'}; }
  }
</script>

<section class="mt-4 border-t border-card-line pt-4" aria-label="Account connections">
  <div class="flex items-center justify-between gap-3">
    <h3 class="text-sm font-medium text-foreground">Account connections</h3>
    <button type="button" class="py-1.5 px-3 text-xs rounded-md bg-muted text-foreground hover:bg-muted-hover disabled:opacity-60" disabled={view.busy} onclick={() => flow.load()}>Refresh connections</button>
  </div>
  <p class="mt-2 text-xs text-muted-foreground-1">Sign in on the host, then choose which sessions can use the connection.</p>
  {#if view.error}<p class="mt-3 text-xs text-destructive-text" role="alert">{view.error}</p>{/if}
  {#if view.catalog}
    <ul class="mt-3 space-y-3">
      {#each view.catalog.scope_profiles as profile (profile.id)}
        <li class="flex items-start justify-between gap-3 text-xs">
          <div class="min-w-0">
            <p class="font-medium text-foreground">{profile.label}</p>
            <p class="mt-1 text-muted-foreground-1">{profile.application_label}</p>
            {#if profile.requested_scopes.length}<p class="mt-1 break-words text-muted-foreground-1">Requested access: {profile.requested_scopes.join(', ')}</p>{/if}
            {#if !profile.enabled || !profile.capabilities.some(c => c.supported)}<p class="mt-1 text-muted-foreground-1">Sign-in is not available on this service.</p>{/if}
          </div>
          <button type="button" class="shrink-0 py-1.5 px-3 rounded-md bg-primary text-primary-foreground hover:bg-primary-hover disabled:opacity-60" disabled={view.busy || pending || !profile.enabled || !profile.capabilities.some(c => c.supported)} onclick={() => flow.authorize(profile.id)}>Sign in</button>
        </li>
      {/each}
    </ul>
    {#if !view.catalog.scope_profiles.length}<p class="mt-3 text-xs text-muted-foreground-1">No sign-in profiles are available.</p>{/if}
    {#each view.catalog.connections as connection (connection.id)}
      {@const profile = view.catalog.scope_profiles.find(p => p.id === connection.scope_profile_id)}
      <div class="mt-4 border-t border-card-line pt-3 text-xs">
        <p class="font-medium text-foreground">{connection.account_label ?? profile?.application_label ?? 'Google account'}</p>
        <p class="mt-1 text-muted-foreground-1">{profile?.label ?? 'Connection'} · {connection.state.replaceAll('_', ' ')}</p>
        {#if connection.provider_revocation !== ProviderRevocationState.NOT_STARTED}<p class="mt-1 text-muted-foreground-1">Provider retirement: {connection.provider_revocation.replaceAll('_', ' ')}</p>{/if}
        <div class="mt-2 flex flex-wrap gap-2">
          <button type="button" class="py-1.5 px-3 rounded-md bg-muted text-foreground hover:bg-muted-hover disabled:opacity-60" disabled={view.busy} onclick={() => flow.check(connection.id)}>Check status</button>
          {#if connection.state !== ConnectionState.CONNECTED && connection.state !== ConnectionState.DISCONNECTING}
            <button type="button" class="py-1.5 px-3 rounded-md bg-primary text-primary-foreground hover:bg-primary-hover disabled:opacity-60" disabled={view.busy || pending || !profile?.enabled || !profile.capabilities.some(c => c.supported)} onclick={() => flow.authorize(connection.scope_profile_id, connection.id)}>Reconnect</button>
          {/if}
          {#if connection.state !== ConnectionState.DISCONNECTED || connection.provider_revocation === ProviderRevocationState.FAILED}
            <button type="button" class="py-1.5 px-3 rounded-md border border-line-2 text-destructive-text hover:bg-muted-hover disabled:opacity-60" disabled={view.busy || connection.state === ConnectionState.DISCONNECTING} onclick={() => flow.disconnect(connection.id)}>Disconnect</button>
          {/if}
        </div>
      </div>
    {/each}
  {:else if view.busy}<p class="mt-3 text-xs text-muted-foreground-1" role="status">Loading connections…</p>{/if}
  {#if view.transaction}
    <div class="mt-4 border-t border-card-line pt-3 text-xs" role="status">
      <p class="text-foreground">Sign-in: {view.transaction.state}</p>
      {#if pending}
        <p class="mt-1 text-muted-foreground-1">Finish signing in in your browser, then check sign-in. Expires {new Date(view.transaction.expires_at).toLocaleString()}.</p>
        <div class="mt-2 flex flex-wrap gap-2">
          {#if view.url}<button type="button" class="py-1.5 px-3 rounded-md bg-primary text-primary-foreground hover:bg-primary-hover" onclick={openSignIn}>Open sign-in</button>{/if}
          <button type="button" class="py-1.5 px-3 rounded-md bg-muted text-foreground hover:bg-muted-hover disabled:opacity-60" disabled={view.busy} onclick={() => flow.poll()}>Check sign-in</button>
          <button type="button" class="py-1.5 px-3 rounded-md border border-line-2 text-foreground disabled:opacity-60" disabled={view.busy} onclick={() => flow.cancel()}>Cancel sign-in</button>
        </div>
      {:else if view.transaction.state === TransactionState.CONNECTED}<p class="mt-1 text-muted-foreground-1">Connected. Grant session access from the session's Credentials action.</p>{/if}
    </div>
  {/if}
</section>
