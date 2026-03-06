<script lang="ts">
  const faqs = [
    {
      question: "Does Capsem work with Claude Code, Gemini CLI, and Codex?",
      answer: "Yes. Capsem supports any AI coding agent that runs in a terminal. Claude Code, Gemini CLI, and Codex are pre-installed in the VM and configured to work through the MITM proxy automatically.",
    },
    {
      question: "How does the MITM proxy work?",
      answer: "All guest HTTPS traffic is redirected through an iptables rule to a local TCP relay, which bridges to the host via vsock. The host terminates TLS using per-domain minted certificates (signed by a static Capsem CA baked into the guest's trust store), inspects the HTTP request, applies policy, and forwards to the real upstream.",
    },
    {
      question: "What platforms are supported?",
      answer: "Capsem requires macOS on Apple Silicon (M1 or later). It uses Apple's Virtualization.framework which is only available on macOS. The guest VM runs aarch64 Linux.",
    },
    {
      question: "Can I customize which domains are allowed?",
      answer: "Yes. Edit ~/.capsem/user.toml to define domain allow/block lists and per-domain HTTP rules (method + path matching). For enterprise deployments, /etc/capsem/corp.toml provides lockdown that individual users cannot override.",
    },
    {
      question: "Is the VM truly air-gapped?",
      answer: "Yes. The guest has no real network interface. It uses a dummy NIC with fake DNS (dnsmasq) and iptables rules that redirect all port 443 traffic through the MITM proxy. Direct IP access and non-443 ports are blocked entirely.",
    },
  ];

  let openIndex = $state<number | null>(1);

  function toggle(i: number) {
    openIndex = openIndex === i ? null : i;
  }
</script>

<section id="faq" class="py-24 md:py-32 bg-surface">
  <div class="mx-auto max-w-6xl px-6">
    <div class="grid md:grid-cols-[1fr_1.5fr] gap-16">
      <div>
        <span class="inline-block rounded-full border border-border bg-badge-bg px-4 py-1.5 text-xs font-medium text-badge-text mb-6">
          FAQ
        </span>
        <h2 class="text-3xl md:text-4xl font-extrabold tracking-tight text-heading">
          Frequently Asked<br />Questions
        </h2>
        <p class="mt-4 text-body">
          Still have a question?
        </p>
        <a href="https://github.com/google/capsem/issues" class="mt-2 inline-block text-accent font-medium hover:underline">
          Open an issue on GitHub
        </a>
      </div>

      <div class="space-y-3">
        {#each faqs as faq, i}
          <div class="rounded-xl border border-border bg-surface-card overflow-hidden">
            <button
              onclick={() => toggle(i)}
              class="w-full flex items-center justify-between p-5 text-left"
            >
              <span class="font-semibold text-heading pr-4">{faq.question}</span>
              <svg
                class="h-5 w-5 text-muted shrink-0 transition-transform duration-200 {openIndex === i ? 'rotate-45' : ''}"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                stroke-width="2"
              >
                <path stroke-linecap="round" stroke-linejoin="round" d="M12 6v12m6-6H6" />
              </svg>
            </button>
            {#if openIndex === i}
              <div class="px-5 pb-5">
                <p class="text-body leading-relaxed">{faq.answer}</p>
              </div>
            {/if}
          </div>
        {/each}
      </div>
    </div>
  </div>
</section>
