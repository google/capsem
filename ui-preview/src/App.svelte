<script lang="ts">
  import A2Node from "./A2Node.svelte";
  import ChatPage from "./ChatPage.svelte";
  import {
    buildSurface,
    type A2uiMessage,
    type PreviewExample,
    type PreviewPayload,
    type PreviewRecipe,
    type RenderAction,
  } from "./a2ui";
  import { onDestroy, onMount } from "svelte";

  type WorkbenchItem = {
    name: string;
    label: string;
    api: string;
    surface: string;
    recipe: PreviewRecipe;
    messages: A2uiMessage[];
    toolRun?: boolean;
    toolResult?: PreviewPayload["toolAcceptance"];
  };

  let payload = $state<PreviewPayload | null>(null);
  let selected = $state(0);
  let inspector = $state<"a2ui" | "tools" | "template">("a2ui");
  let error = $state("");
  let actions = $state<RenderAction[]>([]);
  let refreshTimer: number | undefined;
  const cleanChatPage = window.location.pathname === "/chat";

  let items = $derived.by<WorkbenchItem[]>(() => {
    if (!payload) return [];
    const authoredSurface = payload.authored?.surfaces[0];
    const authoredItem: WorkbenchItem | null = authoredSurface ? {
      name: "Agent Draft",
      label: payload.authored?.ok ? "live" : "invalid",
      api: "POST /ui/tools/run -> stored latest result -> rendered by workbench",
      surface: authoredSurface.surfaceId,
      recipe: authoredSurface.recipe ?? {
        component: "card",
        variant: "simple",
        docsUrl: "https://preline.co/docs/components/card.html",
      },
      messages: authoredSurface.messages,
      toolRun: true,
      toolResult: payload.authored ?? undefined,
    } : null;
    const toolSurface = payload.toolAcceptance.surfaces[0];
    const toolItem: WorkbenchItem = {
      name: "Tool Acceptance",
      label: "UI MCP",
      api: "ui.surface.create -> ui.alert -> ui.surface.validate -> ui.surface.preview",
      surface: toolSurface?.surfaceId ?? "tool-acceptance",
      recipe: {
        component: "alert",
        variant: "discovery",
        docsUrl: "https://preline.co/docs/components/alerts.html#discovery",
      },
      messages: toolSurface?.messages ?? [],
      toolRun: true,
      toolResult: payload.toolAcceptance,
    };

    return [
      ...(authoredItem ? [authoredItem] : []),
      toolItem,
      ...payload.examples.map((example: PreviewExample) => ({
        name: example.name,
        label: example.surface,
        api: example.api,
        surface: example.surface,
        recipe: example.recipe,
        messages: example.messages,
      })),
    ];
  });

  let current = $derived(items[selected]);
  let surface = $derived(current ? buildSurface(current.messages) : null);
  let serialized = $derived(current ? JSON.stringify(current.messages, null, 2) : "");
  let toolSerialized = $derived(
    current?.toolResult ? JSON.stringify(current.toolResult.observations, null, 2) : "",
  );

  onMount(() => {
    loadPayload();
    refreshTimer = window.setInterval(loadPayload, 1500);
  });

  onDestroy(() => {
    if (refreshTimer !== undefined) window.clearInterval(refreshTimer);
  });

  function loadPayload(): void {
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
  }

  function recordAction(action: RenderAction): void {
    actions = [
      {
        ...action,
        context: action.context ?? {},
      },
      ...actions,
    ].slice(0, 6);
  }

  function selectItem(index: number): void {
    selected = index;
    actions = [];
  }
</script>

{#if cleanChatPage}
  <ChatPage />
{:else}
<main class="min-h-screen bg-surface text-foreground">
  <div class="flex min-h-screen">
    <aside class="hidden w-72 shrink-0 border-e border-card-line bg-card lg:block">
      <div class="border-b border-card-line px-5 py-4">
        <p class="text-xs font-semibold uppercase text-muted-foreground-1">Capsem UI</p>
        <h1 class="mt-1 text-lg font-semibold tracking-normal text-foreground">A2UI Workbench</h1>
      </div>
      <nav class="p-3" aria-label="Component workbench">
        {#each items as item, index}
          <button
            type="button"
            class={index === selected
              ? "mb-1 flex w-full items-center justify-between rounded-lg bg-primary px-3 py-2 text-left text-sm font-medium text-primary-foreground"
              : "mb-1 flex w-full items-center justify-between rounded-lg px-3 py-2 text-left text-sm font-medium text-muted-foreground-1 hover:bg-surface-hover hover:text-foreground"}
            onclick={() => selectItem(index)}
          >
            <span>{item.name}</span>
            <span class={index === selected
              ? "rounded-full bg-primary-foreground/15 px-2 py-0.5 text-[11px]"
              : "rounded-full bg-surface px-2 py-0.5 text-[11px] text-muted-foreground"}>
              {item.label}
            </span>
          </button>
        {/each}
      </nav>
    </aside>

    <section class="flex min-w-0 flex-1 flex-col">
      <header class="border-b border-card-line bg-card px-4 py-4 sm:px-6">
        <div class="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
          <div>
            <p class="text-xs font-semibold uppercase text-muted-foreground-1">A2UI v0.9 + checked Preline templates</p>
            <h2 class="mt-1 text-xl font-semibold tracking-normal text-foreground">
              {current?.name ?? "Loading"}
            </h2>
          </div>
          {#if payload}
            <div class="flex flex-wrap items-center gap-2">
              <span class={payload.toolAcceptance.ok
                ? "inline-flex items-center gap-x-1.5 rounded-full bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground"
                : "inline-flex items-center gap-x-1.5 rounded-full bg-destructive px-3 py-1.5 text-xs font-medium text-destructive-foreground"}>
                tool gate {payload.toolAcceptance.ok ? "passing" : "failing"}
              </span>
              <span class="inline-flex items-center gap-x-1.5 rounded-full border border-card-line bg-surface px-3 py-1.5 text-xs font-medium text-muted-foreground-1">
                {payload.catalogId}
              </span>
            </div>
          {/if}
        </div>
      </header>

      {#if error}
        <div class="m-6 rounded-lg border border-destructive bg-destructive/10 p-4 text-sm text-destructive">
          {error}
        </div>
      {:else if !current || !surface}
        <div class="m-6 rounded-lg border border-card-line bg-card p-5 text-sm text-muted-foreground-1 shadow-2xs">
          Loading preview...
        </div>
      {:else}
        <div class="grid flex-1 gap-0 xl:grid-cols-[minmax(0,1fr)_480px]">
          <section class="min-w-0 overflow-auto p-4 sm:p-6">
            <div class="mb-4 flex flex-wrap gap-2 lg:hidden">
              {#each items as item, index}
                <button
                  type="button"
                  class={index === selected
                    ? "rounded-lg bg-primary px-3 py-2 text-sm font-medium text-primary-foreground"
                    : "rounded-lg border border-card-line bg-card px-3 py-2 text-sm font-medium text-muted-foreground-1"}
                  onclick={() => selectItem(index)}
                >
                  {item.name}
                </button>
              {/each}
            </div>

            <article class="overflow-hidden rounded-xl border border-card-line bg-card shadow-2xs">
              <div class="flex items-center justify-between gap-3 border-b border-card-line px-4 py-3">
                <div>
                  <h3 class="text-sm font-semibold tracking-normal text-foreground">Capsem Chat</h3>
                  <p class="mt-1 text-xs text-muted-foreground-1">{current.api}</p>
                </div>
                <span class="inline-flex items-center gap-x-1.5 rounded-full bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground">
                  {current.recipe.component}:{current.recipe.variant}
                </span>
              </div>
              <div class="bg-surface">
                <div class="mx-auto flex min-h-[520px] w-full max-w-3xl flex-col">
                  <div class="flex items-center justify-between border-b border-card-line bg-card px-4 py-3">
                    <div class="flex min-w-0 items-center gap-3">
                      <div class="flex size-9 shrink-0 items-center justify-center rounded-full bg-primary text-sm font-semibold text-primary-foreground">
                        C
                      </div>
                      <div class="min-w-0">
                        <div class="truncate text-sm font-semibold text-foreground">Capsem assistant</div>
                        <div class="truncate text-xs text-muted-foreground-1">UI authoring lane</div>
                      </div>
                    </div>
                    <span class={current.toolRun
                      ? "inline-flex items-center rounded-full bg-primary px-2.5 py-1 text-xs font-medium text-primary-foreground"
                      : "inline-flex items-center rounded-full bg-layer px-2.5 py-1 text-xs font-medium text-layer-foreground"}>
                      {current.label}
                    </span>
                  </div>

                  <div class="flex-1 space-y-5 overflow-auto px-4 py-5">
                    <div class="flex justify-end">
                      <div class="max-w-[78%] rounded-2xl rounded-tr-sm bg-primary px-4 py-3 text-sm text-primary-foreground shadow-2xs">
                        Build me a shell of a chat app and show the generated component inside it.
                      </div>
                    </div>

                    <div class="flex items-start gap-3">
                      <div class="flex size-8 shrink-0 items-center justify-center rounded-full bg-card border border-card-line text-xs font-semibold text-foreground">
                        AI
                      </div>
                      <div class="min-w-0 flex-1">
                        <div class="mb-2 text-xs font-medium text-muted-foreground-1">assistant</div>
                        <div class="rounded-2xl rounded-tl-sm border border-card-line bg-card p-4 shadow-2xs">
                          <p class="mb-4 text-sm text-muted-foreground-1">
                            The component below is rendered from the selected A2UI surface, not handwritten chat HTML.
                          </p>
                          <div class="rounded-xl border border-card-line bg-surface p-4">
                            <A2Node id="root" {surface} recipe={current.recipe} scope={surface.data} onAction={recordAction} />
                          </div>
                        </div>
                      </div>
                    </div>
                  </div>

                  <div class="border-t border-card-line bg-card p-4">
                    <div class="flex items-end gap-3 rounded-xl border border-card-line bg-surface p-2">
                      <textarea
                        class="min-h-11 flex-1 resize-none bg-transparent px-2 py-2 text-sm text-foreground outline-none placeholder:text-muted-foreground-1"
                        rows="1"
                        placeholder="Ask Capsem to create a UI..."
                      ></textarea>
                      <button
                        type="button"
                        class="inline-flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover focus:outline-hidden focus:bg-primary-focus"
                        aria-label="Send"
                      >
                        <svg class="size-4" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                          <path d="m22 2-7 20-4-9-9-4Z"></path>
                          <path d="M22 2 11 13"></path>
                        </svg>
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            </article>

            {#if actions.length}
              <article class="mt-5 rounded-xl border border-card-line bg-card p-4 shadow-2xs">
                <h3 class="text-sm font-semibold tracking-normal text-foreground">Client actions</h3>
                <div class="mt-3 grid gap-2">
                  {#each actions as action, index}
                    <div class="rounded-lg border border-card-line bg-surface p-3 text-xs text-muted-foreground-1">
                      {index + 1}. {action.name} from {action.sourceComponentId}
                    </div>
                  {/each}
                </div>
              </article>
            {/if}
          </section>

          <aside class="border-t border-card-line bg-card xl:border-s xl:border-t-0">
            <div class="border-b border-card-line px-4 py-3">
              <div class="flex gap-2">
                {#each ["a2ui", "tools", "template"] as tab}
                  <button
                    type="button"
                    class={inspector === tab
                      ? "rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-primary-foreground"
                      : "rounded-lg px-3 py-2 text-xs font-semibold text-muted-foreground-1 hover:bg-surface"}
                    onclick={() => (inspector = tab as "a2ui" | "tools" | "template")}
                  >
                    {tab}
                  </button>
                {/each}
              </div>
            </div>
            <div class="p-4">
              {#if inspector === "a2ui"}
                <pre class="max-h-[calc(100vh-180px)] overflow-auto rounded-lg bg-surface p-4 text-xs leading-5 text-foreground">{serialized}</pre>
              {:else if inspector === "tools"}
                <div class="mb-3 rounded-lg border border-card-line bg-surface p-3 text-xs text-muted-foreground-1">
                  Self-use gate: the first surface is built through structured UI tools.
                </div>
                <pre class="max-h-[calc(100vh-235px)] overflow-auto rounded-lg bg-surface p-4 text-xs leading-5 text-foreground">{toolSerialized}</pre>
              {:else}
                <div class="grid gap-3 text-sm">
                  <div class="rounded-lg border border-card-line bg-surface p-3">
                    <div class="text-xs font-semibold uppercase text-muted-foreground-1">Template check</div>
                    <div class="mt-1 text-foreground">component={current.recipe.component}</div>
                    <div class="text-muted-foreground-1">variant={current.recipe.variant}</div>
                  </div>
                  <a
                    class="inline-flex items-center gap-x-1 text-sm font-semibold text-primary hover:text-primary-hover hover:underline"
                    href={current.recipe.docsUrl}
                  >
                    Preline source recipe
                  </a>
                </div>
              {/if}
            </div>
          </aside>
        </div>
      {/if}
    </section>
  </div>
</main>
{/if}
