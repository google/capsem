<script lang="ts">
  import Section from "./Section.svelte";
  import Badge from "./Badge.svelte";
  import Icon from "./Icon.svelte";
  import { FAQS, SITE } from "$lib/data";

  let openIndex = $state<number | null>(1);

  function toggle(i: number) {
    openIndex = openIndex === i ? null : i;
  }
</script>

<Section id="faq">
  <div class="grid md:grid-cols-[1fr_1.5fr] gap-16">
    <div>
      <div class="mb-6">
        <Badge text="FAQ" />
      </div>
      <h2 class="text-3xl md:text-4xl font-extrabold tracking-tight text-heading">
        Frequently Asked<br />Questions
      </h2>
      <p class="mt-4 text-body">Still have a question?</p>
      <a href={SITE.issues} target="_blank" rel="noopener noreferrer" class="mt-2 inline-flex items-center gap-1 text-accent font-medium hover:underline">
        Open an issue on GitHub
        <Icon name="externalLink" class="h-3.5 w-3.5" />
      </a>
    </div>

    <div class="space-y-3" role="list">
      {#each FAQS as faq, i}
        {@const isOpen = openIndex === i}
        <div class="rounded-xl border border-border bg-surface-card overflow-hidden" role="listitem">
          <h3>
            <button
              onclick={() => toggle(i)}
              class="w-full flex items-center justify-between p-5 text-left"
              aria-expanded={isOpen}
              aria-controls="faq-panel-{i}"
              id="faq-btn-{i}"
            >
              <span class="font-semibold text-heading pr-4">{faq.question}</span>
              <Icon
                name="plus"
                class="h-5 w-5 text-muted shrink-0 transition-transform duration-200 {isOpen ? 'rotate-45' : ''}"
              />
            </button>
          </h3>
          <div
            id="faq-panel-{i}"
            role="region"
            aria-labelledby="faq-btn-{i}"
            hidden={!isOpen}
          >
            {#if isOpen}
              <div class="px-5 pb-5">
                <p class="text-body leading-relaxed">{faq.answer}</p>
              </div>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  </div>
</Section>
