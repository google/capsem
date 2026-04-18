<script lang="ts">
  import Section from "./Section.svelte";
  import SectionHeader from "./SectionHeader.svelte";
  import Card from "./Card.svelte";
  import Icon from "./Icon.svelte";
  import { AGENTS, MCP_TOOLS, PACKAGES, ROADMAP } from "$lib/data";
</script>

<Section id="features">
  <SectionHeader
    title="Ready to code, out of the box"
    subtitle="A fully configured Linux dev environment with your favorite AI agents, MCP tools, and packages pre-installed. Boot to prompt in under 10 seconds."
  />

  <div class="grid md:grid-cols-2 gap-6">
    <Card title="Broad agent support" subtitle="All major AI coding agents, pre-installed and auto-configured for sandbox mode.">
      <div class="space-y-3">
        {#each AGENTS as agent}
          <div class="flex items-center justify-between rounded-xl border border-border px-4 py-3">
            <span class="font-medium text-heading">{agent.name}</span>
            <span class="text-xs text-muted">{agent.provider}</span>
          </div>
        {/each}
      </div>
    </Card>

    <Card title="Native MCP gateway" subtitle="Policy-checked tool access over vsock. Built-in tools plus custom MCP servers via fastmcp.">
      <div class="space-y-3">
        {#each MCP_TOOLS as tool}
          <div class="flex items-center justify-between rounded-xl border border-border px-4 py-3">
            <code class="text-sm font-mono text-accent">{tool.name}</code>
            <span class="text-xs text-muted">{tool.desc}</span>
          </div>
        {/each}
      </div>
    </Card>

    <Card title="Preconfigured environment" subtitle="Python, Node.js, and 30+ packages ready to go. No setup required.">
      <div class="flex flex-wrap gap-2">
        {#each PACKAGES as pkg}
          <span class="rounded-lg border border-border px-2.5 py-1 text-xs font-mono text-body">{pkg}</span>
        {/each}
      </div>
    </Card>

    <Card title="Coming soon" subtitle="Active development. Here's what's next." class="flex flex-col">
      <ul class="space-y-3 mb-8">
        {#each ROADMAP as item}
          <li class="flex items-start gap-3">
            <Icon name="clock" class="h-5 w-5 text-accent-secondary shrink-0 mt-0.5" />
            <span class="text-body text-sm">{item}</span>
          </li>
        {/each}
      </ul>
      <div class="mt-auto rounded-xl border border-dashed border-border bg-surface-alt flex items-center justify-center py-10">
        <div class="text-center text-muted">
          <Icon name="play" class="h-10 w-10 mx-auto mb-2 opacity-40" />
          <span class="text-xs">Demo video</span>
        </div>
      </div>
    </Card>
  </div>
</Section>
