<script lang="ts">
  import A2Node from "./A2Node.svelte";
  import {
    buildSurface,
    type PreviewExample,
    type PreviewPayload,
    type RenderAction,
  } from "./a2ui";

  let payload: PreviewPayload | null = null;
  let selected = 0;
  let error = "";
  let actions: RenderAction[] = [];

  $: examples = payload?.examples ?? [];
  $: current = examples[selected] as PreviewExample | undefined;
  $: surface = current ? buildSurface(current.messages) : null;
  $: serialized = current ? JSON.stringify(current.messages, null, 2) : "";

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

<main class="min-h-screen bg-slate-50">
  <div class="mx-auto flex max-w-7xl flex-col gap-5 px-4 py-5 sm:px-6 lg:px-8">
    <header class="flex flex-col gap-3 border-b border-slate-200 pb-4 lg:flex-row lg:items-end lg:justify-between">
      <div>
        <p class="text-xs font-semibold uppercase tracking-normal text-slate-500">Capsem UI pipeline</p>
        <h1 class="text-2xl font-semibold tracking-normal text-slate-950">A2UI Basic preview</h1>
      </div>
      {#if payload}
        <div class="rounded-md border border-slate-200 bg-white px-3 py-2 text-xs text-slate-600">
          {payload.generatedBy}
        </div>
      {/if}
    </header>

    {#if error}
      <section class="rounded-lg border border-red-200 bg-red-50 p-4 text-sm text-red-800">
        {error}
      </section>
    {:else if !current || !surface}
      <section class="rounded-lg border border-slate-200 bg-white p-5 text-sm text-slate-600">
        Loading preview...
      </section>
    {:else}
      <div class="flex flex-wrap gap-2">
        {#each examples as example, index}
          <button
            type="button"
            class={index === selected
              ? "rounded-md border border-slate-950 bg-slate-950 px-3 py-2 text-sm font-medium text-white"
              : "rounded-md border border-slate-200 bg-white px-3 py-2 text-sm font-medium text-slate-700 hover:bg-slate-100"}
            on:click={() => {
              selected = index;
              actions = [];
            }}
          >
            {example.name}
          </button>
        {/each}
      </div>

      <section class="grid gap-4 lg:grid-cols-[minmax(0,0.85fr)_minmax(0,1fr)_minmax(0,1fr)]">
        <article class="rounded-lg border border-slate-200 bg-white p-4">
          <div class="mb-3 flex items-center justify-between gap-3">
            <h2 class="text-sm font-semibold tracking-normal text-slate-900">Rust API</h2>
            <span class="rounded-md bg-slate-100 px-2 py-1 text-xs text-slate-600">{current.surface}</span>
          </div>
          <pre class="overflow-auto rounded-md bg-slate-950 p-3 text-xs leading-5 text-slate-50">{current.api}</pre>
          <div class="mt-4 rounded-md border border-slate-200 bg-slate-50 p-3">
            <p class="text-xs font-semibold text-slate-600">Catalog</p>
            <p class="mt-1 break-all text-xs text-slate-500">{payload?.catalogId}</p>
          </div>
          {#if actions.length}
            <div class="mt-4">
              <h3 class="mb-2 text-xs font-semibold text-slate-600">Client actions</h3>
              <div class="flex flex-col gap-2">
                {#each actions as action, index}
                  <div class="rounded-md border border-slate-200 bg-slate-50 p-2 text-xs text-slate-700">
                    {index + 1}. {action.name} from {action.sourceComponentId}
                  </div>
                {/each}
              </div>
            </div>
          {/if}
        </article>

        <article class="rounded-lg border border-slate-200 bg-white p-4">
          <h2 class="mb-3 text-sm font-semibold tracking-normal text-slate-900">A2UI Basic format</h2>
          <pre class="max-h-[640px] overflow-auto rounded-md bg-slate-950 p-3 text-xs leading-5 text-slate-50">{serialized}</pre>
        </article>

        <article class="rounded-lg border border-slate-200 bg-white p-4">
          <h2 class="mb-3 text-sm font-semibold tracking-normal text-slate-900">Svelte/Preline renderer</h2>
          <div class="rounded-lg border border-slate-200 bg-slate-50 p-4">
            <A2Node id="root" {surface} scope={surface.data} onAction={recordAction} />
          </div>
        </article>
      </section>
    {/if}
  </div>
</main>
