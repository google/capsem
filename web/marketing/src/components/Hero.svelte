<script lang="ts">
  import { SITE, UI } from "$lib/data";
  import InstallCommand from "./InstallCommand.svelte";
  import HeroDemo from "./HeroDemo.svelte";
</script>
<section class="hero section-space" aria-labelledby="hero-title">
  <div class="content-shell grid items-start gap-10 lg:grid-cols-[0.85fr_1.15fr] lg:gap-12">
    <div>
      <h1 id="hero-title" class="text-3xl leading-[1.1] font-bold tracking-tight sm:text-4xl lg:text-5xl"><span class="brand-gradient">{SITE.name.toUpperCase()}</span><br />{SITE.tagline}</h1>
      <div class="mt-8 mb-4 flex items-center gap-4"><a class="button" href={SITE.releases}>{UI.download}<span aria-hidden="true">↓</span></a><span class="max-w-40 text-xs leading-5 text-body">{SITE.platform}</span></div>
      <InstallCommand />
    </div>
    <HeroDemo />
  </div>
</section>
