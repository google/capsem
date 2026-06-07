<script lang="ts">
  import type { ProviderStatus, ToolConfigSourceRecord } from '../../types/settings';
  import Brain from 'phosphor-svelte/lib/Brain';
  import CheckCircle from 'phosphor-svelte/lib/CheckCircle';
  import FileText from 'phosphor-svelte/lib/FileText';
  import Key from 'phosphor-svelte/lib/Key';
  import ShieldWarning from 'phosphor-svelte/lib/ShieldWarning';

  let {
    providers = [],
    toolConfigSources = {},
  }: {
    providers?: ProviderStatus[];
    toolConfigSources?: Record<string, ToolConfigSourceRecord>;
  } = $props();

  let sourceEntries = $derived(Object.entries(toolConfigSources));
  let discoveredCount = $derived(providers.filter((provider) => provider.discovery).length);
  let brokeredCount = $derived(providers.filter((provider) => provider.brokered_credential_ref).length);

  function shortRef(ref: string | null | undefined): string {
    if (!ref) return '';
    const marker = 'credential:blake3:';
    if (ref.startsWith(marker)) {
      return `${marker}${ref.slice(-12)}`;
    }
    return ref.length > 28 ? `${ref.slice(0, 12)}...${ref.slice(-12)}` : ref;
  }

  function formatOverlay(value: string): string {
    return value.replace(/_/g, ' ');
  }
</script>

{#if providers.length > 0 || sourceEntries.length > 0}
  <section class="mb-6">
    <div class="flex items-center justify-between gap-3 mb-2">
      <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider">Provider Runtime</h3>
      <div class="flex items-center gap-2 text-[11px] text-muted-foreground-1">
        <span class="inline-flex items-center gap-1">
          <Brain size={13} />
          {discoveredCount}/{providers.length} discovered
        </span>
        <span class="inline-flex items-center gap-1">
          <Key size={13} />
          {brokeredCount} brokered
        </span>
      </div>
    </div>

    {#if providers.length > 0}
      <div class="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
        {#each providers as provider (provider.id)}
          <article class="bg-card border border-card-line rounded-lg p-4">
            <div class="flex items-start justify-between gap-3">
              <div class="min-w-0">
                <p class="text-sm font-medium text-foreground truncate">{provider.name}</p>
                <p class="text-xs text-muted-foreground-1 truncate">
                  {provider.protocol ?? provider.id}{#if provider.url} - {provider.url}{/if}
                </p>
              </div>
              {#if provider.corp_blocked}
                <span class="inline-flex items-center gap-1 rounded-md border border-destructive/20 bg-destructive/10 px-2 py-1 text-[11px] font-medium text-destructive">
                  <ShieldWarning size={13} />
                  Blocked
                </span>
              {:else if provider.brokered_credential_ref}
                <span class="inline-flex items-center gap-1 rounded-md border border-primary/20 bg-primary/10 px-2 py-1 text-[11px] font-medium text-primary">
                  <Key size={13} />
                  Brokered
                </span>
              {:else if provider.discovery}
                <span class="inline-flex items-center gap-1 rounded-md border border-line-2 bg-layer px-2 py-1 text-[11px] font-medium text-foreground">
                  <CheckCircle size={13} />
                  Detected
                </span>
              {:else}
                <span class="rounded-md border border-line-2 bg-layer px-2 py-1 text-[11px] font-medium text-muted-foreground-1">
                  Configured
                </span>
              {/if}
            </div>

            <dl class="mt-3 space-y-1.5 text-xs">
              {#if provider.discovery}
                <div class="flex items-center justify-between gap-3">
                  <dt class="text-muted-foreground-1">Source</dt>
                  <dd class="text-foreground truncate">{provider.discovery.source}</dd>
                </div>
                <div class="flex items-center justify-between gap-3">
                  <dt class="text-muted-foreground-1">Event</dt>
                  <dd class="text-foreground truncate">{provider.discovery.event_type ?? 'unknown'}</dd>
                </div>
              {/if}
              {#if provider.brokered_credential_ref}
                <div class="flex items-center justify-between gap-3">
                  <dt class="text-muted-foreground-1">Credential</dt>
                  <dd class="font-mono text-[11px] text-foreground truncate">{shortRef(provider.brokered_credential_ref)}</dd>
                </div>
              {/if}
              {#if provider.discovery?.trace_id}
                <div class="flex items-center justify-between gap-3">
                  <dt class="text-muted-foreground-1">Trace</dt>
                  <dd class="font-mono text-[11px] text-foreground truncate">{provider.discovery.trace_id}</dd>
                </div>
              {/if}
            </dl>
          </article>
        {/each}
      </div>
    {/if}

    {#if sourceEntries.length > 0}
      <div class="mt-4 bg-card border border-card-line rounded-lg divide-y divide-card-divider">
        {#each sourceEntries as [key, source] (key)}
          <div class="p-4">
            <div class="flex items-start justify-between gap-3">
              <div class="min-w-0">
                <p class="inline-flex items-center gap-2 text-sm font-medium text-foreground">
                  <FileText size={16} />
                  {source.tool_id}
                </p>
                <p class="mt-1 font-mono text-xs text-muted-foreground-1 truncate">{source.guest_path}</p>
              </div>
              <span class="rounded-md border border-line-2 bg-layer px-2 py-1 text-[11px] font-medium text-muted-foreground-1">
                {source.format}
              </span>
            </div>
            <div class="mt-3 grid gap-2 text-xs sm:grid-cols-2">
              {#if source.inferred_endpoint_ref}
                <div>
                  <p class="text-muted-foreground-1">Provider</p>
                  <p class="text-foreground">{source.inferred_endpoint_ref}</p>
                </div>
              {/if}
              {#if source.observed_hash}
                <div>
                  <p class="text-muted-foreground-1">Hash</p>
                  <p class="font-mono text-[11px] text-foreground truncate">{source.observed_hash}</p>
                </div>
              {/if}
              {#if source.credential_refs.length > 0}
                <div>
                  <p class="text-muted-foreground-1">Credentials</p>
                  <p class="font-mono text-[11px] text-foreground truncate">{source.credential_refs.map(shortRef).join(', ')}</p>
                </div>
              {/if}
              {#if source.allowed_overlays.length > 0}
                <div>
                  <p class="text-muted-foreground-1">Overlays</p>
                  <p class="text-foreground truncate">{source.allowed_overlays.map(formatOverlay).join(', ')}</p>
                </div>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </section>
{/if}
