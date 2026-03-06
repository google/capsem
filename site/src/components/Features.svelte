<script lang="ts">
  const agents = [
    { name: "Claude Code", provider: "Anthropic" },
    { name: "Gemini CLI", provider: "Google" },
    { name: "Codex", provider: "OpenAI" },
  ];

  const mcpTools = [
    { name: "fetch_http", desc: "Fetch and extract web content" },
    { name: "grep_http", desc: "Search web pages with regex" },
    { name: "http_headers", desc: "Inspect HTTP headers and status" },
  ];

  const packages = [
    "Python 3", "Node.js 24", "git", "uv",
    "numpy", "pandas", "scipy", "scikit-learn",
    "requests", "httpx", "beautifulsoup4",
    "pytest", "rich", "matplotlib", "fastmcp",
  ];

  const roadmap = [
    "VM checkpointing and restore",
    "Linux host support",
    "VS Code extension",
    "Custom MCP server marketplace",
  ];
</script>

<section id="features" class="py-24 md:py-32 bg-surface">
  <div class="mx-auto max-w-6xl px-6">
    <div class="text-center mb-16">
      <h2 class="text-4xl md:text-5xl font-extrabold tracking-tight text-heading">
        Ready to code, out of the box
      </h2>
      <p class="mt-4 text-lg text-body max-w-2xl mx-auto">
        A fully configured Linux dev environment with your favorite AI agents,
        MCP tools, and packages pre-installed. Boot to prompt in under 10 seconds.
      </p>
    </div>

    <div class="grid md:grid-cols-2 gap-6">
      <!-- Agent support -->
      <div class="rounded-2xl border border-border bg-surface-card p-8">
        <h3 class="text-xl font-bold text-heading mb-1">Broad agent support</h3>
        <p class="text-sm text-muted mb-6">All major AI coding agents, pre-installed and auto-configured for sandbox mode.</p>
        <div class="space-y-3">
          {#each agents as agent}
            <div class="flex items-center justify-between rounded-xl border border-border px-4 py-3">
              <span class="font-medium text-heading">{agent.name}</span>
              <span class="text-xs text-muted">{agent.provider}</span>
            </div>
          {/each}
        </div>
      </div>

      <!-- Native MCP -->
      <div class="rounded-2xl border border-border bg-surface-card p-8">
        <h3 class="text-xl font-bold text-heading mb-1">Native MCP gateway</h3>
        <p class="text-sm text-muted mb-6">Policy-checked tool access over vsock. Built-in tools plus custom MCP servers via fastmcp.</p>
        <div class="space-y-3">
          {#each mcpTools as tool}
            <div class="flex items-center justify-between rounded-xl border border-border px-4 py-3">
              <code class="text-sm font-mono text-accent">{tool.name}</code>
              <span class="text-xs text-muted">{tool.desc}</span>
            </div>
          {/each}
        </div>
      </div>

      <!-- Preconfigured environment -->
      <div class="rounded-2xl border border-border bg-surface-card p-8">
        <h3 class="text-xl font-bold text-heading mb-1">Preconfigured environment</h3>
        <p class="text-sm text-muted mb-6">Python, Node.js, and 30+ packages ready to go. No setup required.</p>
        <div class="flex flex-wrap gap-2">
          {#each packages as pkg}
            <span class="rounded-lg border border-border px-2.5 py-1 text-xs font-mono text-body">{pkg}</span>
          {/each}
        </div>
      </div>

      <!-- Roadmap + video -->
      <div class="rounded-2xl border border-border bg-surface-card p-8 flex flex-col">
        <h3 class="text-xl font-bold text-heading mb-1">Coming soon</h3>
        <p class="text-sm text-muted mb-6">Active development. Here's what's next.</p>
        <ul class="space-y-3 mb-8">
          {#each roadmap as item}
            <li class="flex items-start gap-3">
              <svg class="h-5 w-5 text-accent-secondary shrink-0 mt-0.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M12 6v6l4 2" /><circle cx="12" cy="12" r="10" /></svg>
              <span class="text-body text-sm">{item}</span>
            </li>
          {/each}
        </ul>
        <!-- Video placeholder -->
        <div class="mt-auto rounded-xl border border-dashed border-border bg-surface-alt flex items-center justify-center py-10">
          <div class="text-center text-muted">
            <svg class="h-10 w-10 mx-auto mb-2 opacity-40" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5"><polygon points="5 3 19 12 5 21 5 3" /></svg>
            <span class="text-xs">Demo video</span>
          </div>
        </div>
      </div>
    </div>
  </div>
</section>
