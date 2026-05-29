<script lang="ts">
  import A2Node from "./A2Node.svelte";
  import {
    buildSurface,
    type PreviewExample,
    type PreviewPayload,
    type RenderAction,
  } from "./a2ui";
  import { onMount } from "svelte";

  let payload = $state<PreviewPayload | null>(null);
  let selected = $state(0);
  let error = $state("");
  let actions = $state<RenderAction[]>([]);

  let examples = $derived(payload?.examples ?? []);
  let current = $derived(examples[selected] as PreviewExample | undefined);
  let surface = $derived(current ? buildSurface(current.messages) : null);
  let serialized = $derived(current ? JSON.stringify(current.messages, null, 2) : "");

  onMount(() => {
    fetch("/ui/spec/demo")
      .then((response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return response.json();
      })
      .then((body: PreviewPayload) => {
        payload = body;
      })
      .catch((cause: Error) => {
        error = cause.message;
      });
  });

  function recordAction(action: RenderAction): void {
    actions = [
      {
        ...action,
        context: action.context ?? {},
      },
      ...actions,
    ].slice(0, 5);
  }
</script>

<main class="min-h-screen bg-background">
  <div class="mx-auto flex max-w-[85rem] flex-col gap-6 px-4 py-6 sm:px-6 lg:px-8">
    <header class="flex flex-col gap-5 lg:flex-row lg:items-end lg:justify-between">
      <div class="max-w-2xl">
        <span class="inline-flex items-center gap-x-1.5 rounded-full border border-primary/15 bg-primary-100 px-3 py-1.5 text-xs font-medium text-primary">
          A2UI v0.9 Basic
        </span>
        <h1 class="mt-4 text-3xl font-semibold tracking-normal text-foreground">Capsem UI renderer</h1>
        <p class="mt-2 text-sm leading-6 text-muted-foreground-1">
          Rust emits validated A2UI Basic messages; Svelte renders them with Preline component patterns.
        </p>
      </div>
      {#if payload}
        <div class="rounded-xl border border-layer-line bg-layer px-4 py-3 text-xs text-muted-foreground-1 shadow-2xs">
          {payload.generatedBy}
        </div>
      {/if}
    </header>

    {#if error}
      <section class="rounded-xl border border-card-line bg-card p-4 text-sm text-warning">
        {error}
      </section>
    {:else if !current || !surface}
      <section class="rounded-xl border border-card-line bg-card p-5 text-sm text-muted-foreground-1">
        Loading preview...
      </section>
    {:else}
      <div class="rounded-xl border border-layer-line bg-layer p-1.5 shadow-2xs">
        <div class="flex flex-wrap gap-1.5">
          {#each examples as example, index}
            <button
              type="button"
              class={index === selected
                ? "rounded-lg border border-primary bg-primary px-3 py-2 text-sm font-medium text-primary-foreground shadow-2xs"
                : "rounded-lg border border-transparent px-3 py-2 text-sm font-medium text-layer-foreground hover:bg-layer-hover"}
              onclick={() => {
                selected = index;
                actions = [];
              }}
            >
              {example.name}
            </button>
          {/each}
        </div>
      </div>

      <section class="grid gap-5 lg:grid-cols-[minmax(0,1.35fr)_minmax(360px,0.75fr)]">
        <article class="rounded-xl border border-card-line bg-card shadow-2xs">
          <div class="border-b border-card-divider px-4 py-3 md:px-5">
            <div class="flex items-center justify-between gap-3">
              <h2 class="text-sm font-semibold tracking-normal text-foreground">Svelte/Preline renderer</h2>
              <span class="inline-flex items-center gap-x-1.5 rounded-full bg-primary-100 px-2.5 py-1 text-xs font-medium text-primary">
                rendered first
              </span>
            </div>
          </div>
          <div class="bg-surface p-4 md:p-5">
            <div class="mx-auto max-w-xl">
              <A2Node id="root" {surface} scope={surface.data} onAction={recordAction} />
            </div>
          </div>
        </article>

        <article class="rounded-xl border border-card-line bg-card p-4 shadow-2xs md:p-5">
          <div class="mb-3 flex items-center justify-between gap-3">
            <h2 class="text-sm font-semibold tracking-normal text-foreground">Rust API</h2>
            <span class="rounded-lg bg-surface px-2 py-1 text-xs text-muted-foreground-1">{current.surface}</span>
          </div>
          <pre class="overflow-auto rounded-lg bg-surface p-3 text-xs leading-5 text-foreground">{current.api}</pre>
          <div class="mt-4 rounded-lg border border-card-line bg-surface p-3">
            <p class="text-xs font-semibold text-foreground">Catalog</p>
            <p class="mt-1 break-all text-xs text-muted-foreground-1">{payload?.catalogId}</p>
          </div>
          {#if actions.length}
            <div class="mt-4">
              <h3 class="mb-2 text-xs font-semibold text-foreground">Client actions</h3>
              <div class="flex flex-col gap-2">
                {#each actions as action, index}
                  <div class="rounded-lg border border-card-line bg-surface p-2 text-xs text-muted-foreground-1">
                    {index + 1}. {action.name} from {action.sourceComponentId}
                  </div>
                {/each}
              </div>
            </div>
          {/if}
        </article>
      </section>

      <section class="rounded-xl border border-card-line bg-card shadow-2xs">
        <div class="border-b border-card-divider px-4 py-3 md:px-5">
          <h2 class="text-sm font-semibold tracking-normal text-foreground">A2UI Basic format</h2>
        </div>
        <div class="p-4 md:p-5">
          <pre class="max-h-[520px] overflow-auto rounded-lg bg-surface p-4 text-xs leading-5 text-foreground">{serialized}</pre>
        </div>
      </section>
    {/if}
  </div>
</main>
