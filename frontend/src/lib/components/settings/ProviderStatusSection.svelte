<script lang="ts">
  import type { ProviderStatus } from '../../types/settings';
  import Brain from 'phosphor-svelte/lib/Brain';
  import CheckCircle from 'phosphor-svelte/lib/CheckCircle';
  import ShieldWarning from 'phosphor-svelte/lib/ShieldWarning';

  let {
    providers = [],
  }: {
    providers?: ProviderStatus[];
  } = $props();

  let discoveredCount = $derived(providers.filter((provider) => provider.discovery).length);
</script>

{#if providers.length > 0}
  <section class="mb-6">
    <div class="flex items-center justify-between gap-3 mb-2">
      <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider">Provider Runtime</h3>
      <div class="flex items-center gap-2 text-[11px] text-muted-foreground-1">
        <span class="inline-flex items-center gap-1">
          <Brain size={13} />
          {discoveredCount}/{providers.length} discovered
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
              {:else if provider.discovery}
                <span class="inline-flex items-center gap-1 rounded-md border border-line-2 bg-layer px-2 py-1 text-[11px] font-medium text-foreground">
                  <CheckCircle size={13} />
                  Detected
                </span>
              {:else}
                <span class="rounded-md border border-line-2 bg-layer px-2 py-1 text-[11px] font-medium text-muted-foreground-1">
                  Endpoint
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

  </section>
{/if}
