<script lang="ts">
  let scrolled = $state(false);

  function onScroll() {
    scrolled = window.scrollY > 20;
  }

  $effect(() => {
    window.addEventListener("scroll", onScroll);
    return () => window.removeEventListener("scroll", onScroll);
  });
</script>

<nav
  class="fixed top-0 left-0 right-0 z-50 transition-all duration-300 {scrolled
    ? 'bg-surface/80 backdrop-blur-xl border-b border-border shadow-sm'
    : 'bg-transparent'}"
>
  <div class="mx-auto max-w-6xl flex items-center justify-between px-6 py-4">
    <a href="/" class="flex items-center gap-2.5 group">
      <img src="/logo.svg" alt="Capsem" class="h-8 w-8" />
      <span class="text-lg font-bold text-heading tracking-tight">Capsem</span>
    </a>

    <div class="hidden md:flex items-center gap-8">
      <a href="/#features" class="text-sm text-body hover:text-heading transition-colors">Features</a>
      <a href="/#how-it-works" class="text-sm text-body hover:text-heading transition-colors">How It Works</a>
      <a href="/news" class="text-sm text-body hover:text-heading transition-colors">News</a>
      <a href="/documentation/security/kernel" class="text-sm text-body hover:text-heading transition-colors">Docs</a>
    </div>

    <div class="flex items-center gap-3">
      <a
        href="#download"
        class="inline-flex items-center gap-2 rounded-full bg-gradient-to-r from-accent to-accent-secondary text-white px-5 py-2 text-sm font-medium hover:opacity-90 transition-opacity"
      >
        <svg class="h-4 w-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M4 16v2a2 2 0 002 2h12a2 2 0 002-2v-2M7 10l5 5 5-5M12 15V3"/></svg>
        Download
      </a>
      <a
        href="https://github.com/google/capsem"
        target="_blank"
        rel="noopener noreferrer"
        class="inline-flex items-center gap-2 rounded-full bg-heading text-surface px-5 py-2 text-sm font-medium hover:bg-heading/90 transition-colors"
      >
        <svg class="h-4 w-4" fill="currentColor" viewBox="0 0 24 24"><path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z"/></svg>
        GitHub
      </a>
    </div>
  </div>
</nav>
