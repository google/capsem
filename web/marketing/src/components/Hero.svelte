<script lang="ts">
  import { SITE } from "$lib/data";
  import Download from "./Download.svelte";
  import InstallCommand from "./InstallCommand.svelte";
  import HeroDemo from "./HeroDemo.svelte";
</script>
<section class="hero section-space" aria-labelledby="hero-title">
  <div class="content-shell grid items-stretch gap-10 lg:grid-cols-[0.85fr_1.15fr] lg:gap-12">
    <div class="flex flex-col items-center gap-8 text-center lg:gap-0">
      <p class="brand-gradient mx-auto w-fit py-4 text-6xl font-bold tracking-tight sm:text-7xl lg:py-8 lg:text-8xl">{SITE.name.toUpperCase()}</p>
      <div class="flex flex-1 items-center py-4 lg:py-10"><h1 id="hero-title" class="text-3xl leading-[1.1] font-bold tracking-tight sm:text-4xl lg:text-5xl">{SITE.tagline}</h1></div>
      <div class="w-full space-y-4">
        <Download />
        <InstallCommand />
      </div>
    </div>
    <HeroDemo />
  </div>
</section>
