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
  <div class="mx-auto flex max-w-7xl flex-col gap-5 px-4 py-5 sm:px-6 lg:px-8">
    <header class="flex flex-col gap-3 border-b border-card-divider pb-4 lg:flex-row lg:items-end lg:justify-between">
      <div>
        <p class="text-xs font-semibold uppercase tracking-normal text-muted-foreground-1">Capsem UI pipeline</p>
        <h1 class="text-2xl font-semibold tracking-normal text-foreground">A2UI Basic preview</h1>
      </div>
      {#if payload}
        <div class="rounded-lg border border-layer-line bg-layer px-3 py-2 text-xs text-muted-foreground-1">
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
      <div class="flex flex-wrap gap-2">
        {#each examples as example, index}
          <button
            type="button"
            class={index === selected
              ? "rounded-lg border border-primary bg-primary px-3 py-2 text-sm font-medium text-primary-foreground"
              : "rounded-lg border border-layer-line bg-layer px-3 py-2 text-sm font-medium text-layer-foreground hover:bg-layer-hover"}
            onclick={() => {
              selected = index;
              actions = [];
            }}
          >
            {example.name}
          </button>
        {/each}
      </div>

      <section class="grid gap-4 lg:grid-cols-[minmax(0,0.85fr)_minmax(0,1fr)_minmax(0,1fr)]">
        <article class="rounded-xl border border-card-line bg-card p-4 shadow-2xs">
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

        <article class="rounded-xl border border-card-line bg-card p-4 shadow-2xs">
          <h2 class="mb-3 text-sm font-semibold tracking-normal text-foreground">A2UI Basic format</h2>
          <pre class="max-h-[640px] overflow-auto rounded-lg bg-surface p-3 text-xs leading-5 text-foreground">{serialized}</pre>
        </article>

        <article class="rounded-xl border border-card-line bg-card p-4 shadow-2xs">
          <h2 class="mb-3 text-sm font-semibold tracking-normal text-foreground">Svelte/Preline renderer</h2>
          <div class="rounded-xl border border-card-line bg-surface p-4">
            <A2Node id="root" {surface} scope={surface.data} onAction={recordAction} />
          </div>
        </article>
      </section>
    {/if}
  </div>
</main>
