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
  <div class="mx-auto flex max-w-5xl flex-col gap-6 px-4 py-6 sm:px-6 lg:px-8">
    <header class="flex flex-col gap-3">
      <div class="max-w-2xl">
        <span class="inline-flex items-center gap-x-1.5 rounded-full border border-primary/15 bg-primary-100 px-3 py-1.5 text-xs font-medium text-primary">
          A2UI v0.9 Basic
        </span>
        <h1 class="mt-4 text-2xl font-semibold tracking-normal text-foreground">Capsem UI renderer</h1>
        <p class="mt-2 text-sm leading-6 text-muted-foreground-1">
          Rust emits validated A2UI Basic messages; Svelte maps their fields into Preline component recipes.
        </p>
      </div>
    </header>

    {#if error}
      <section class="rounded-xl border border-card-line bg-card p-4 text-sm text-destructive">
        {error}
      </section>
    {:else if !current || !surface}
      <section class="rounded-xl border border-card-line bg-card p-5 text-sm text-muted-foreground-1">
        Loading preview...
      </section>
    {:else}
      <div class="border-b border-line-2">
        <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
        <nav class="flex gap-x-2 overflow-x-auto" aria-label="Tabs" role="tablist" aria-orientation="horizontal">
          {#each examples as example, index}
            <button
              type="button"
              role="tab"
              aria-selected={index === selected}
              class={index === selected
                ? "active -mb-px py-3 px-4 inline-flex items-center gap-x-2 bg-layer text-sm font-medium text-center border border-line-2 border-b-transparent text-primary-active rounded-t-lg focus:outline-hidden focus:text-primary-focus disabled:opacity-50 disabled:pointer-events-none"
                : "-mb-px py-3 px-4 inline-flex items-center gap-x-2 bg-muted text-sm font-medium text-center border border-line-2 text-muted-foreground-1 rounded-t-lg hover:text-foreground focus:outline-hidden focus:text-foreground disabled:opacity-50 disabled:pointer-events-none"}
              onclick={() => {
                selected = index;
                actions = [];
              }}
            >
              {example.name}
            </button>
          {/each}
        </nav>
      </div>

      <section class="rounded-xl border border-card-line bg-card shadow-2xs">
        <div class="border-b border-card-divider px-4 py-3">
          <div class="flex items-center justify-between gap-3">
            <h2 class="text-sm font-semibold tracking-normal text-foreground">Preline recipe render</h2>
            <span class="inline-flex items-center gap-x-1.5 py-1.5 px-3 rounded-full text-xs font-medium bg-primary text-primary-foreground">
              {current.surface}
            </span>
          </div>
        </div>
        <div class="p-6">
          <div class="mx-auto w-full max-w-xl">
            <A2Node id="root" {surface} scope={surface.data} onAction={recordAction} />
          </div>
        </div>
      </section>

      <section class="grid gap-5 lg:grid-cols-2">
        <article class="rounded-xl border border-card-line bg-card p-4 shadow-2xs">
          <div class="mb-3 flex items-center justify-between gap-3">
            <h2 class="text-sm font-semibold tracking-normal text-foreground">Rust API</h2>
            <span class="inline-flex items-center gap-x-1.5 py-1.5 px-3 rounded-full text-xs font-medium bg-primary-100 text-primary-800">
              {payload?.generatedBy}
            </span>
          </div>
          <pre class="overflow-auto rounded-lg bg-surface p-3 text-xs leading-5 text-foreground">{current.api}</pre>
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

        <article class="rounded-xl border border-card-line bg-card shadow-2xs">
          <div class="border-b border-card-divider px-4 py-3">
            <div class="flex items-center justify-between gap-3">
              <h2 class="text-sm font-semibold tracking-normal text-foreground">A2UI Basic format</h2>
              <span class="inline-flex items-center gap-x-1.5 py-1.5 px-3 rounded-full text-xs font-medium bg-layer border border-layer-line text-layer-foreground">
                validated
              </span>
            </div>
          </div>
          <div class="p-4">
            <pre class="max-h-[360px] overflow-auto rounded-lg bg-surface p-4 text-xs leading-5 text-foreground">{serialized}</pre>
          </div>
        </article>
      </section>
    {/if}
  </div>
</main>
