<script lang="ts">
  import { HERO, SITE, UI } from "$lib/data";
  import InstallCommand from "./InstallCommand.svelte";
  import HeroDemo from "./HeroDemo.svelte";
</script>
<section class="hero container" aria-labelledby="hero-title">
  <div class="hero-heading">
    <h1 id="hero-title">{HERO.lines[0]}<br />{HERO.lines[1]}<br /><span>{HERO.lines[2]}</span></h1>
    <div class="hero-intro">
      <h2>{HERO.title}</h2>
      <p>{HERO.description}</p>
      <div class="hero-actions"><a class="button" href={SITE.releases}>{UI.download}<span aria-hidden="true">↓</span></a><span class="platform-note">{SITE.platform}</span></div>
      <InstallCommand />
    </div>
  </div>
  <HeroDemo />
</section>
