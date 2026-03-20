<section id="download" class="section-dark py-24 md:py-32 relative overflow-hidden">
  <div class="absolute inset-0 bg-[radial-gradient(ellipse_60%_50%_at_50%_-20%,var(--color-accent-glow),transparent)]"></div>

  <div class="relative mx-auto max-w-4xl px-6 text-center">
    <h2 class="text-4xl md:text-6xl font-extrabold tracking-tight text-heading-dark leading-tight">
      Download Capsem
    </h2>
    <p class="mt-6 text-lg text-body-dark max-w-xl mx-auto">
      Open source, native macOS, boots in under 10 seconds.
    </p>

    <!-- Install one-liner (dark theme) -->
    <div class="mt-10 mx-auto max-w-lg flex items-center gap-3 rounded-xl border border-border-dark bg-surface-dark-alt/60 px-5 py-3.5 font-mono text-sm text-heading-dark select-all">
      <span class="text-muted-dark select-none">$</span>
      <span>curl -fsSL https://capsem.org/install.sh | sh</span>
      <button
        onclick={() => navigator.clipboard.writeText('curl -fsSL https://capsem.org/install.sh | sh')}
        class="ml-auto shrink-0 rounded-md p-1.5 text-muted-dark hover:text-heading-dark hover:bg-surface-dark transition-colors"
        aria-label="Copy to clipboard"
      >
        <svg class="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>
      </button>
    </div>

    <p class="mt-4 text-sm text-muted-dark">or download the DMG directly</p>

    <div class="mt-4 flex flex-wrap justify-center gap-4">
      <a
        href="https://github.com/google/capsem/releases/latest"
        class="inline-flex items-center gap-2 rounded-full bg-gradient-to-r from-accent to-accent-secondary text-white px-7 py-3.5 text-sm font-semibold hover:opacity-90 transition-opacity"
      >
        <svg class="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M4 16v2a2 2 0 002 2h12a2 2 0 002-2v-2M12 3v12m0 0l-4-4m4 4l4-4" /></svg>
        Download DMG
      </a>
      <a
        href="https://github.com/google/capsem"
        target="_blank"
        rel="noopener noreferrer"
        class="inline-flex items-center gap-2 rounded-full border border-border-dark px-7 py-3.5 text-sm font-semibold text-heading-dark hover:bg-surface-dark-alt transition-colors"
      >
        View on GitHub
      </a>
    </div>

    <p class="mt-8 text-xs text-muted-dark">
      Requires macOS 14+ on Apple Silicon. Capsem uses Apple Virtualization.framework.
    </p>
  </div>
</section>
