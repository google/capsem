<script lang="ts">
  import Icon from "./Icon.svelte";
  import InstallCommand from "./InstallCommand.svelte";
  import { SITE } from "$lib/data";
</script>

<section id="download" class="section-dark py-24 md:py-32 relative overflow-hidden">
  <div class="absolute inset-0 bg-[radial-gradient(ellipse_60%_50%_at_50%_-20%,var(--color-accent-glow),transparent)]"></div>

  <div class="relative mx-auto max-w-4xl px-6 text-center">
    <h2 class="text-4xl md:text-6xl font-extrabold tracking-tight text-heading-dark leading-tight">
      Download {SITE.name}
    </h2>
    <p class="mt-6 text-lg text-body-dark max-w-xl mx-auto">
      Open source, native macOS, boots in under 10 seconds.
    </p>

    <div class="mt-10 mx-auto max-w-lg">
      <InstallCommand dark />
    </div>

    <p class="mt-4 text-sm text-muted-dark">or download the DMG directly</p>

    <div class="mt-4 flex flex-wrap justify-center gap-4">
      <a href={SITE.releases} target="_blank" rel="noopener noreferrer" class="btn-primary">
        <Icon name="downloadAlt" />
        Download DMG
        <span class="sr-only">(opens in new tab)</span>
      </a>
      <a href={SITE.github} target="_blank" rel="noopener noreferrer" class="btn-outline-dark">
        View on GitHub
        <span class="sr-only">(opens in new tab)</span>
      </a>
    </div>

    <p class="mt-8 text-xs text-muted-dark">
      Requires {SITE.platform}. Capsem uses Apple Virtualization.framework.
    </p>
  </div>
</section>
