<script lang="ts">
  import Icon from "./Icon.svelte";
  import { SITE, NAV_LINKS } from "$lib/data";

  let scrolled = $state(false);
  let mobileOpen = $state(false);

  $effect(() => {
    const onScroll = () => { scrolled = window.scrollY > 20; };
    window.addEventListener("scroll", onScroll);
    return () => window.removeEventListener("scroll", onScroll);
  });

  function closeMobile() {
    mobileOpen = false;
  }
</script>

<nav
  aria-label="Main navigation"
  class="fixed top-0 left-0 right-0 z-50 transition-all duration-300 {scrolled || mobileOpen
    ? 'bg-surface/80 backdrop-blur-xl border-b border-border shadow-sm'
    : 'bg-transparent'}"
>
  <div class="mx-auto max-w-6xl flex items-center justify-between px-6 py-4">
    <a href="/" class="flex items-center gap-2.5">
      <img src="/logo.svg" alt="" class="h-8 w-8" />
      <span class="text-lg font-bold text-heading tracking-tight">{SITE.name}</span>
    </a>

    <!-- Desktop links -->
    <div class="hidden md:flex items-center gap-8">
      {#each NAV_LINKS as link}
        <a href={link.href} class="text-sm text-body hover:text-heading transition-colors">{link.label}</a>
      {/each}
    </div>

    <div class="flex items-center gap-3">
      <a href="#download" class="btn-primary hidden sm:inline-flex">
        <Icon name="download" />
        Download
      </a>
      <a href={SITE.github} target="_blank" rel="noopener noreferrer" class="btn-dark hidden sm:inline-flex">
        <Icon name="github" />
        <span>GitHub</span>
        <span class="sr-only">(opens in new tab)</span>
      </a>

      <!-- Mobile menu toggle -->
      <button
        class="md:hidden p-2 text-heading"
        onclick={() => mobileOpen = !mobileOpen}
        aria-expanded={mobileOpen}
        aria-controls="mobile-menu"
        aria-label={mobileOpen ? "Close menu" : "Open menu"}
      >
        {#if mobileOpen}
          <Icon name="x" class="h-6 w-6" />
        {:else}
          <Icon name="menu" class="h-6 w-6" />
        {/if}
      </button>
    </div>
  </div>

  <!-- Mobile menu -->
  {#if mobileOpen}
    <div id="mobile-menu" class="md:hidden border-t border-border bg-surface/95 backdrop-blur-xl">
      <div class="mx-auto max-w-6xl px-6 py-4 flex flex-col gap-3">
        {#each NAV_LINKS as link}
          <a
            href={link.href}
            class="text-sm text-body hover:text-heading transition-colors py-2"
            onclick={closeMobile}
          >{link.label}</a>
        {/each}
        <div class="flex gap-3 pt-2">
          <a href="#download" class="btn-primary" onclick={closeMobile}>
            <Icon name="download" />
            Download
          </a>
          <a href={SITE.github} target="_blank" rel="noopener noreferrer" class="btn-dark">
            <Icon name="github" />
            GitHub
          </a>
        </div>
      </div>
    </div>
  {/if}
</nav>
