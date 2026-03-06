<script>
  const blocks = [
    {
      badge: "ISOLATION",
      title: "Hardware-level sandboxing with Apple Virtualization.framework",
      description:
        "Each agent session boots a lightweight Linux VM with a read-only rootfs, no swap, no kernel modules, no debugfs. Air-gapped networking with a dummy NIC and fake DNS ensures nothing reaches the real network without going through the MITM proxy.",
      bullets: [
        "Ephemeral VMs -- fresh state every session",
        "Read-only rootfs with tmpfs workspace",
        "No systemd, no sshd, no cron -- minimal attack surface",
      ],
    },
    {
      badge: "INSPECTION",
      title: "See everything your AI agent does on the network",
      description:
        "A transparent MITM proxy terminates TLS from the guest using per-domain minted certificates, inspects every HTTP request and response, and applies policy before forwarding to the real upstream. Full request/response bodies are logged to a per-session SQLite database.",
      bullets: [
        "Per-domain TLS certificate minting",
        "Method + path policy rules per domain",
        "Full body capture for post-hoc analysis",
      ],
    },
    {
      badge: "CONTROL",
      title: "Enterprise-grade policy with user and corp config layers",
      description:
        "User-level config in ~/.capsem/user.toml lets developers customize domain lists and HTTP rules. Corp-level config at /etc/capsem/corp.toml (MDM-distributed) locks down policy with enterprise overrides that users cannot bypass.",
      bullets: [
        "Domain allow/block with wildcard support",
        "HTTP method + path matching per domain",
        "Corp config overrides user config entirely",
      ],
    },
  ];
</script>

<section id="security" class="py-24 md:py-32 bg-surface-alt">
  <div class="mx-auto max-w-6xl px-6">
    <div class="text-center mb-20">
      <h2 class="text-4xl md:text-5xl font-extrabold tracking-tight text-heading">
        Security without compromise
      </h2>
    </div>

    <div class="space-y-16">
      {#each blocks as block, i}
        <div class="flex flex-col {i % 2 === 1 ? 'md:flex-row-reverse' : 'md:flex-row'} gap-12 items-center">
          <div class="flex-1">
            <span class="inline-block rounded-full border border-border bg-badge-bg px-3 py-1 text-xs font-medium text-badge-text mb-4">
              {block.badge}
            </span>
            <h3 class="text-2xl md:text-3xl font-bold text-heading leading-tight mb-4">
              {block.title}
            </h3>
            <p class="text-body leading-relaxed mb-6">{block.description}</p>
            <ul class="space-y-3">
              {#each block.bullets as bullet}
                <li class="flex items-start gap-3">
                  <svg class="h-5 w-5 text-accent shrink-0 mt-0.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7" /></svg>
                  <span class="text-body">{bullet}</span>
                </li>
              {/each}
            </ul>
          </div>

          <div class="flex-1">
            <div class="relative rounded-2xl border border-border bg-surface-card p-1 shadow-lg">
              <div class="rounded-xl bg-gradient-to-br from-accent/5 to-accent-secondary/5 p-8 min-h-[280px] flex items-center justify-center">
                <div class="text-center text-muted text-sm">
                  <!-- Placeholder for screenshot / illustration -->
                  <svg class="h-16 w-16 mx-auto mb-3 opacity-30" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1"><rect x="3" y="3" width="18" height="18" rx="2" /><path d="M3 9h18M9 21V9" /></svg>
                  Screenshot placeholder
                </div>
              </div>
            </div>
          </div>
        </div>
      {/each}
    </div>
  </div>
</section>
