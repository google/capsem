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

<main class="min-h-screen bg-white dark:bg-neutral-900">
  <div class="mx-auto flex max-w-5xl flex-col gap-6 px-4 py-6 sm:px-6 lg:px-8">
    <header class="flex flex-col gap-3">
      <div class="max-w-2xl">
        <span class="inline-flex items-center gap-x-1.5 py-1.5 px-3 rounded-full text-xs font-medium bg-primary-100 text-primary-800 dark:bg-primary-500/20 dark:text-primary-400">
          A2UI v0.9 Basic
        </span>
        <h1 class="mt-4 text-2xl font-semibold tracking-normal text-stone-800 dark:text-neutral-200">Capsem UI renderer</h1>
        <p class="mt-2 text-sm leading-6 text-stone-500 dark:text-neutral-400">
          Rust emits validated A2UI Basic messages; Svelte maps their fields into Preline component recipes.
        </p>
      </div>
    </header>

    {#if error}
      <section class="rounded-xl border border-red-200 bg-red-50 p-4 text-sm text-red-800">
        {error}
      </section>
    {:else if !current || !surface}
      <section class="rounded-xl border border-stone-200 bg-white p-5 text-sm text-stone-500 shadow-2xs dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-400">
        Loading preview...
      </section>
    {:else}
      <div class="border-b border-stone-200 dark:border-neutral-700">
        <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
        <nav class="flex gap-x-2 overflow-x-auto" aria-label="Tabs" role="tablist" aria-orientation="horizontal">
          {#each examples as example, index}
            <button
              type="button"
              role="tab"
              aria-selected={index === selected}
              class={index === selected
                ? "active -mb-px py-3 px-4 inline-flex items-center gap-x-2 bg-white text-sm font-medium text-center border border-stone-200 border-b-transparent text-primary rounded-t-lg focus:outline-hidden focus:text-primary-focus disabled:opacity-50 disabled:pointer-events-none dark:bg-neutral-900 dark:border-neutral-700 dark:text-primary"
                : "-mb-px py-3 px-4 inline-flex items-center gap-x-2 bg-stone-50 text-sm font-medium text-center border border-stone-200 text-stone-500 rounded-t-lg hover:text-stone-700 focus:outline-hidden focus:text-stone-700 disabled:opacity-50 disabled:pointer-events-none dark:bg-neutral-800 dark:border-neutral-700 dark:text-neutral-400 dark:hover:text-neutral-200"}
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

      <section class="rounded-xl border border-stone-200 bg-white shadow-2xs dark:border-neutral-700 dark:bg-neutral-900">
        <div class="border-b border-stone-200 px-4 py-3 dark:border-neutral-700">
          <div class="flex items-center justify-between gap-3">
            <h2 class="text-sm font-semibold tracking-normal text-stone-800 dark:text-neutral-200">Preline recipe render</h2>
            <span class="inline-flex items-center gap-x-1.5 py-1.5 px-3 rounded-full text-xs font-medium bg-primary text-primary-foreground">
              {current.recipe.component}:{current.recipe.variant}
            </span>
          </div>
        </div>
        <div class="p-6">
          <div class="mx-auto w-full max-w-xl">
            <A2Node id="root" {surface} recipe={current.recipe} scope={surface.data} onAction={recordAction} />
          </div>
        </div>
      </section>

      <section class="grid gap-5 lg:grid-cols-2">
        <article class="rounded-xl border border-stone-200 bg-white p-4 shadow-2xs dark:border-neutral-700 dark:bg-neutral-900">
          <div class="mb-3 flex items-center justify-between gap-3">
            <h2 class="text-sm font-semibold tracking-normal text-stone-800 dark:text-neutral-200">Rust API</h2>
            <span class="inline-flex items-center gap-x-1.5 py-1.5 px-3 rounded-full text-xs font-medium bg-primary-100 text-primary-800 dark:bg-primary-500/20 dark:text-primary-400">
              {payload?.generatedBy}
            </span>
          </div>
          <pre class="overflow-auto rounded-lg bg-stone-100 p-3 text-xs leading-5 text-stone-800 dark:bg-neutral-800 dark:text-neutral-200">{current.api}</pre>
          {#if actions.length}
            <div class="mt-4">
              <h3 class="mb-2 text-xs font-semibold text-stone-800 dark:text-neutral-200">Client actions</h3>
              <div class="flex flex-col gap-2">
                {#each actions as action, index}
                  <div class="rounded-lg border border-stone-200 bg-stone-100 p-2 text-xs text-stone-500 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-400">
                    {index + 1}. {action.name} from {action.sourceComponentId}
                  </div>
                {/each}
              </div>
            </div>
          {/if}
        </article>

        <article class="rounded-xl border border-stone-200 bg-white shadow-2xs dark:border-neutral-700 dark:bg-neutral-900">
          <div class="border-b border-stone-200 px-4 py-3 dark:border-neutral-700">
            <div class="flex items-center justify-between gap-3">
              <h2 class="text-sm font-semibold tracking-normal text-stone-800 dark:text-neutral-200">A2UI Basic format</h2>
              <span class="inline-flex items-center gap-x-1.5 py-1.5 px-3 rounded-full text-xs font-medium bg-white border border-stone-200 text-stone-800 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-200">
                validated
              </span>
            </div>
          </div>
          <div class="p-4">
            <pre class="max-h-[360px] overflow-auto rounded-lg bg-stone-100 p-4 text-xs leading-5 text-stone-800 dark:bg-neutral-800 dark:text-neutral-200">{serialized}</pre>
          </div>
        </article>
      </section>
    {/if}
  </div>
</main>
