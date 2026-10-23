<script lang="ts">
  import type { CredentialBrokerInfo } from '../../api';
  import type { VmSummary } from '@capsem/sdk';
  let { info, sessions = [] }: { info: CredentialBrokerInfo; sessions?: VmSummary[] } = $props();
  const groups = $derived.by(() => {
    const known = info.sessions ?? [];
    const associated = new Set(known.flatMap(group => group.inventory.map(row => row.credential_ref)));
    const unknown = info.inventory.filter(row => !associated.has(row.credential_ref));
    return [...known, ...(!info.sessions || unknown.length ? [{ vm_id: null, inventory: unknown }] : [])];
  });
</script>

<div class="space-y-4">
  <p class="text-xs text-muted-foreground-1">Observed credential metadata by VM. An observation does not grant access to another session.</p>
  {#if !groups.length}<p class="text-sm text-muted-foreground-1">No credential activity recorded.</p>{/if}
  {#each groups as group (group.vm_id ?? 'unknown')}
    <section class="min-w-0" aria-label={group.vm_id ? `Credentials observed by ${group.vm_id}` : 'VM association unavailable'}>
      <h3 class="text-sm font-medium text-foreground mb-2">{group.vm_id ? sessions.find(vm => vm.id === group.vm_id)?.name ?? group.vm_id : 'VM association unavailable'}</h3>
      {#if group.inventory.length}
        <div class="overflow-x-auto rounded-lg border border-line-2">
          <table class="w-full text-left text-xs">
            <thead class="text-muted-foreground-2"><tr><th class="px-3 py-2 font-medium">Provider</th><th class="px-3 py-2 font-medium">Source</th><th class="px-3 py-2 font-medium">Last observed</th></tr></thead>
            <tbody class="divide-y divide-card-divider">
              {#each group.inventory as credential (credential.credential_ref)}
                <tr><td class="px-3 py-2 text-foreground">{credential.provider ?? 'Unknown provider'}</td><td class="px-3 py-2 text-muted-foreground-1">{credential.source ?? 'Not recorded'}</td><td class="px-3 py-2 text-muted-foreground-1">{credential.last_seen ?? 'Not recorded'}</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
      {:else}<p class="text-xs text-muted-foreground-1">No credential activity recorded for this VM.</p>{/if}
    </section>
  {/each}
</div>
