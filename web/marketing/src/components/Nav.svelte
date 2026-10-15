<script lang="ts">
  import { NAV_LINKS, SITE, UI } from "$lib/data";
  let open = $state(false);
</script>
<svelte:window onkeydown={(e) => { if (e.key === "Escape") open = false; }} />
<header class="site-header">
  <nav class="container nav" aria-label="Main navigation">
    <a class="brand" href="/" aria-label={SITE.name}><img src="/logo.svg" alt="" width="32" height="32" />{SITE.name}<span class="brand-dot">.</span></a>
    <div class="nav-links">
      {#each NAV_LINKS as link}<a href={link.href}>{link.label}</a>{/each}
    </div>
    <a class="nav-github" href={SITE.github}>{UI.source}<span aria-hidden="true"> ↗</span></a>
    <button class="menu-button" onclick={() => open = !open} aria-expanded={open} aria-controls="mobile-nav" aria-label={open ? UI.closeMenu : UI.openMenu}>
      <span></span><span></span>
    </button>
    {#if open}
      <div class="mobile-nav" id="mobile-nav">
        {#each NAV_LINKS as link}<a onclick={() => open = false} href={link.href}>{link.label}<span aria-hidden="true">↗</span></a>{/each}
      </div>
    {/if}
  </nav>
</header>
