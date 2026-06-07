<script lang="ts">
  import Section from "./Section.svelte";
  import SectionHeader from "./SectionHeader.svelte";
  import Icon from "./Icon.svelte";
  import { HOST_COMPONENTS, GUEST_COMPONENTS, VSOCK_CHANNELS } from "$lib/data";
  import type { IconName } from "$lib/icons";
</script>

{#snippet archCard(label: string, detail: string, icon: string, accent: "primary" | "secondary")}
  <div class="rounded-xl border border-border-dark bg-surface-dark p-5">
    <div class="flex items-center gap-3">
      <div class="h-8 w-8 rounded-lg {accent === 'primary' ? 'bg-accent/15' : 'bg-accent-secondary/15'} flex items-center justify-center">
        <Icon name={icon as IconName} class="h-4 w-4 {accent === 'primary' ? 'text-accent' : 'text-accent-secondary'}" />
      </div>
      <div>
        <div class="text-sm font-semibold text-heading-dark">{label}</div>
        <div class="text-xs text-muted-dark">{detail}</div>
      </div>
    </div>
  </div>
{/snippet}

<Section id="how-it-works" dark>
  <SectionHeader
    title="How it works"
    subtitle="A native macOS hypervisor creates an air-gapped Linux VM for each session. All network traffic is forced through an inspecting proxy on the host."
    dark
  />

  <div class="rounded-2xl border border-border-dark bg-surface-dark-alt p-8 md:p-12">
    <div class="grid md:grid-cols-[1fr_auto_1fr] gap-8 items-stretch">

      <!-- Host side -->
      <div class="space-y-4">
        <div class="text-xs font-semibold uppercase tracking-wider text-muted-dark mb-2">macOS Host</div>
        {#each HOST_COMPONENTS as c}
          {@render archCard(c.label, c.detail, c.icon, "primary")}
        {/each}
      </div>

      <!-- Connection arrows -->
      <div class="hidden md:flex flex-col items-center justify-center gap-4 py-8">
        {#each VSOCK_CHANNELS as ch, i}
          {#if i > 0}
            <div class="w-px h-4 bg-border-dark"></div>
          {/if}
          <div class="flex flex-col items-center gap-1">
            {#if i === 0}
              <span class="text-[10px] text-muted-dark font-mono">vsock</span>
              <div class="w-px h-6 bg-border-dark"></div>
            {/if}
            <span class="text-[10px] text-muted-dark font-mono">{ch.port}</span>
            <Icon name="bidir" class="h-4 w-4 {i === 0 ? 'text-accent' : 'text-accent-secondary'}" />
            <span class="text-[10px] text-muted-dark font-mono">{ch.label}</span>
          </div>
        {/each}
      </div>

      <!-- Guest side -->
      <div class="space-y-4">
        <div class="text-xs font-semibold uppercase tracking-wider text-muted-dark mb-2">Linux VM (air-gapped)</div>
        {#each GUEST_COMPONENTS as c}
          {@render archCard(c.label, c.detail, c.icon, "secondary")}
        {/each}
      </div>
    </div>

    <!-- Bottom: Internet -->
    <div class="mt-8 pt-8 border-t border-border-dark text-center">
      <div class="inline-flex items-center gap-3 rounded-xl border border-border-dark bg-surface-dark px-6 py-3">
        <Icon name="globe" class="h-5 w-5 text-muted-dark" />
        <span class="text-sm text-muted-dark">Internet (via host MITM proxy only)</span>
      </div>
    </div>
  </div>
</Section>
