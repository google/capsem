<script lang="ts">
  import { SITE } from "$lib/data";
  import Download from "./Download.svelte";
  import InstallCommand from "./InstallCommand.svelte";
  import HeroDemo from "./HeroDemo.svelte";
</script>
<section class="hero section-space" aria-labelledby="hero-title">
  <div class="content-shell grid items-stretch gap-10 lg:grid-cols-[0.85fr_1.15fr] lg:gap-12">
    <div class="flex flex-col justify-center gap-8 lg:gap-10">
      <p class="brand-gradient w-fit text-6xl font-bold leading-none tracking-tight sm:text-7xl lg:text-8xl">{SITE.name.toUpperCase()}</p>
      <div><h1 id="hero-title" class="text-3xl leading-[1.1] font-bold tracking-tight sm:text-4xl lg:text-5xl">{SITE.tagline}</h1></div>
      <div class="w-full space-y-4">
        <Download />
        <InstallCommand />
      </div>
    </div>
    <HeroDemo />
  </div>
</section>
