<script lang="ts">
  import HeroDemo from "./HeroDemo.svelte";
  import { HERO_DEMO, SITE } from "$lib/data";
  import InstallCommand from "./InstallCommand.svelte";
</script>

<section class="relative pt-32 pb-20 md:pt-44 md:pb-32 overflow-hidden">
  <div class="absolute inset-0 bg-[linear-gradient(to_right,#e5e7eb_1px,transparent_1px),linear-gradient(to_bottom,#e5e7eb_1px,transparent_1px)] bg-[size:4rem_4rem] opacity-40"></div>
  <div class="absolute inset-0 bg-[radial-gradient(ellipse_60%_50%_at_50%_-20%,var(--color-accent-glow),transparent)]"></div>

  <div class="relative mx-auto max-w-[1440px] px-6 hero-grid">
    <div class="hero-copy">
      <p class="hero-eyebrow">{HERO_DEMO.label}</p>
      <h1 class="text-5xl lg:text-6xl font-extrabold tracking-tight text-heading leading-[1.08]">
        <span class="gradient-text">{SITE.name}</span><br />
        {HERO_DEMO.title}
      </h1>

      <p class="mt-6 text-lg md:text-xl text-body max-w-xl leading-relaxed">
        {HERO_DEMO.description}
      </p>

      <div class="mt-10 flex flex-col gap-4 max-w-lg">
        <InstallCommand />
        <div class="flex flex-wrap gap-4">
          <a href="#features" class="btn-outline">Learn More</a>
        </div>
      </div>
    </div>
    <HeroDemo />
  </div>
</section>
