<script lang="ts">
  import { onMount } from 'svelte';
  import { getCredentialBrokerInfo, type CredentialBrokerInfo } from '../../api';
  import { vmStore } from '../../stores/vms.svelte';
  import { watchVisibleRefresh } from '../../models/visible-refresh';
  import CredentialList from './CredentialList.svelte';
  let info = $state<CredentialBrokerInfo | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  async function load() {
    if (busy) return;
    busy = true;
    try { info = await getCredentialBrokerInfo(); error = null; }
    catch (cause) { error = cause instanceof Error ? cause.message : 'Credential metadata unavailable'; }
    finally { busy = false; }
  }
  onMount(() => { void load(); return watchVisibleRefresh(load, 30_000); });
</script>

<h2 class="text-sm font-medium text-foreground mb-3">Credentials</h2>
{#if error}
  <div class="text-sm text-destructive-text mb-3" role="alert">{error}
    <button type="button" class="ml-2 underline disabled:opacity-50" disabled={busy} onclick={load}>Retry</button>
  </div>
{/if}
{#if info}<CredentialList {info} sessions={vmStore.vms} />
{:else if !error}<p class="text-sm text-muted-foreground-1" role="status">Loading credential metadata…</p>{/if}
