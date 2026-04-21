<script lang="ts">
  import Icon from "./Icon.svelte";
  import { SITE, FOOTER_COLUMNS } from "$lib/data";
</script>

<footer class="section-dark border-t border-border-dark" role="contentinfo">
  <div class="mx-auto max-w-6xl px-6 py-16">
    <div class="flex flex-col md:flex-row justify-between gap-12">
      <div>
        <a href="/" class="flex items-center gap-2.5">
          <img src="/logo.svg" alt="" class="h-7 w-7 invert" />
          <span class="text-lg font-bold text-heading-dark tracking-tight">{SITE.name}</span>
        </a>
        <p class="mt-3 text-sm text-muted-dark max-w-xs">
          {SITE.tagline}
        </p>
        <div class="mt-6">
          <a href={SITE.github} target="_blank" rel="noopener noreferrer" class="text-muted-dark hover:text-heading-dark transition-colors">
            <Icon name="github" class="h-5 w-5" label="GitHub repository" />
          </a>
        </div>
      </div>

      <div class="grid grid-cols-2 gap-12">
        {#each FOOTER_COLUMNS as column}
          <div>
            <h3 class="text-sm font-semibold text-heading-dark mb-4">{column.title}</h3>
            <ul class="space-y-2.5">
              {#each column.links as link}
                {@const isExternal = link.href.startsWith("http")}
                <li>
                  <a
                    href={link.href}
                    class="text-sm text-muted-dark hover:text-heading-dark transition-colors"
                    target={isExternal ? "_blank" : undefined}
                    rel={isExternal ? "noopener noreferrer" : undefined}
                  >
                    {link.label}
                    {#if isExternal}<span class="sr-only">(opens in new tab)</span>{/if}
                  </a>
                </li>
              {/each}
            </ul>
          </div>
        {/each}
      </div>
    </div>

    <div class="mt-12 pt-8 border-t border-border-dark flex flex-col md:flex-row justify-between items-center gap-4">
      <p class="text-sm text-muted-dark">
        &copy; {new Date().getFullYear()} {SITE.copyright}. Open source under {SITE.license} license.
      </p>
    </div>
  </div>
</footer>
