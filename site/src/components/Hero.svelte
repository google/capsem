<section class="relative pt-32 pb-20 md:pt-44 md:pb-32 overflow-hidden">
  <!-- Subtle grid background -->
  <div class="absolute inset-0 bg-[linear-gradient(to_right,#e5e7eb_1px,transparent_1px),linear-gradient(to_bottom,#e5e7eb_1px,transparent_1px)] bg-[size:4rem_4rem] opacity-40"></div>
  <!-- Radial fade -->
  <div class="absolute inset-0 bg-[radial-gradient(ellipse_60%_50%_at_50%_-20%,var(--color-accent-glow),transparent)]"></div>

  <div class="relative mx-auto max-w-6xl px-6">
    <div class="max-w-3xl">
      <h1 class="text-5xl md:text-7xl font-extrabold tracking-tight text-heading leading-[1.08]">
        <span class="gradient-text">CAPSEM</span><br />
        Native AI Agent<br />
        Security
      </h1>

      <p class="mt-6 text-lg md:text-xl text-body max-w-xl leading-relaxed">
        A Rust-powered hypervisor that sandboxes every AI coding agent
        in its own air-gapped Linux VM. See everything, control everything.
      </p>

      <!-- Install one-liner -->
      <div class="mt-10 flex flex-col gap-4 max-w-lg">
        <div class="group relative flex items-center gap-3 rounded-xl border border-border bg-surface-alt/60 px-5 py-3.5 font-mono text-sm text-heading select-all">
          <span class="text-muted select-none">$</span>
          <span>curl -fsSL https://capsem.org/install.sh | sh</span>
          <button
            onclick={() => navigator.clipboard.writeText('curl -fsSL https://capsem.org/install.sh | sh')}
            class="ml-auto shrink-0 rounded-md p-1.5 text-muted hover:text-heading hover:bg-surface transition-colors"
            aria-label="Copy to clipboard"
          >
            <svg class="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>
          </button>
        </div>
        <div class="flex flex-wrap gap-4">
          <a
            href="#features"
            class="inline-flex items-center gap-2 rounded-full border border-border px-7 py-3.5 text-sm font-semibold text-heading hover:bg-surface-alt transition-colors"
          >
            Learn More
          </a>
        </div>
      </div>
    </div>
  </div>
</section>
