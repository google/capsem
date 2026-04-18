<script lang="ts">
  import Section from "./Section.svelte";
  import SectionHeader from "./SectionHeader.svelte";
  import Badge from "./Badge.svelte";
  import Icon from "./Icon.svelte";
  import { SECURITY_BLOCKS } from "$lib/data";
</script>

<Section id="security" alt>
  <SectionHeader title="Security without compromise" />

  <div class="space-y-16">
    {#each SECURITY_BLOCKS as block, i}
      <div class="flex flex-col {i % 2 === 1 ? 'md:flex-row-reverse' : 'md:flex-row'} gap-12 items-center">
        <div class="flex-1">
          <div class="mb-4">
            <Badge text={block.badge} />
          </div>
          <h3 class="text-2xl md:text-3xl font-bold text-heading leading-tight mb-4">
            {block.title}
          </h3>
          <p class="text-body leading-relaxed mb-6">{block.description}</p>
          <ul class="space-y-3">
            {#each block.bullets as bullet}
              <li class="flex items-start gap-3">
                <Icon name="check" class="h-5 w-5 text-accent shrink-0 mt-0.5" />
                <span class="text-body">{bullet}</span>
              </li>
            {/each}
          </ul>
        </div>

        <div class="flex-1">
          <div class="relative rounded-2xl border border-border bg-surface-card p-1 shadow-lg">
            <div class="rounded-xl bg-gradient-to-br from-accent/5 to-accent-secondary/5 p-8 min-h-[280px] flex items-center justify-center">
              <div class="text-center text-muted text-sm">
                <Icon name="image" class="h-16 w-16 mx-auto mb-3 opacity-30" />
                Screenshot placeholder
              </div>
            </div>
          </div>
        </div>
      </div>
    {/each}
  </div>
</Section>
