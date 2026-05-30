<script lang="ts">
  import A2Node from "./A2Node.svelte";
  import { buildSurface, type A2uiMessage, type PreviewPayload, type PreviewRecipe, type RenderAction } from "./a2ui";
  import { onDestroy, onMount } from "svelte";

  type ChatSurface = {
    label: string;
    surfaceId: string;
    recipe: PreviewRecipe;
    messages: A2uiMessage[];
  };

  const themes = [
    { label: "Default", value: "default" },
    { label: "Ocean", value: "theme-ocean" },
    { label: "Moon", value: "theme-moon" },
    { label: "Olive", value: "theme-olive" },
    { label: "Bubblegum", value: "theme-bubblegum" },
    { label: "Autumn", value: "theme-autumn" },
    { label: "Cashmere", value: "theme-cashmere" },
    { label: "Harvest", value: "theme-harvest" },
    { label: "Retro", value: "theme-retro" },
  ];

  let payload = $state<PreviewPayload | null>(null);
  let error = $state("");
  let actions = $state<RenderAction[]>([]);
  let selectedTheme = $state("default");
  let darkMode = $state(false);
  let refreshTimer: number | undefined;

  let chatSurfaces = $derived.by<ChatSurface[]>(() => {
    const authored = payload?.authored?.surfaces ?? [];
    if (authored.length > 0) {
      return authored.map((surface) => ({
        label: payload?.authored?.ok ? "live" : "invalid",
        surfaceId: surface.surfaceId,
        recipe: surface.recipe ?? {
          component: "card",
          variant: "simple",
          docsUrl: "https://preline.co/docs/components/card.html",
        },
        messages: surface.messages,
      }));
    }

    const fallback = payload?.toolAcceptance.surfaces[0];
    if (!fallback) return [];
    return [{
      label: "demo",
      surfaceId: fallback.surfaceId,
      recipe: {
        component: "alert",
        variant: "discovery",
        docsUrl: "https://preline.co/docs/components/alerts.html#discovery",
      },
      messages: fallback.messages,
    }];
  });

  let renderedSurfaces = $derived(
    chatSurfaces.map((chatSurface) => ({
      ...chatSurface,
      surface: buildSurface(chatSurface.messages),
    })),
  );

  onMount(() => {
    selectedTheme = window.localStorage.getItem("capsem-chat-theme") ?? "default";
    darkMode = window.localStorage.getItem("capsem-chat-dark") === "true";
    applyTheme();
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
    actions = [{ ...action, context: action.context ?? {} }, ...actions].slice(0, 4);
  }

  function applyTheme(): void {
    const root = document.documentElement;
    if (selectedTheme === "default") {
      root.removeAttribute("data-theme");
    } else {
      root.dataset.theme = selectedTheme;
    }
    root.classList.toggle("dark", darkMode);
  }

  function updateTheme(): void {
    window.localStorage.setItem("capsem-chat-theme", selectedTheme);
    window.localStorage.setItem("capsem-chat-dark", String(darkMode));
    applyTheme();
  }
</script>

<main class="min-h-screen bg-surface text-foreground">
  <section class="mx-auto flex min-h-screen w-full max-w-4xl flex-col bg-card shadow-2xs">
    <header class="flex flex-wrap items-center justify-between gap-3 border-b border-card-line px-4 py-3">
      <div class="flex min-w-0 items-center gap-3">
        <div class="flex size-9 shrink-0 items-center justify-center rounded-full bg-primary text-sm font-semibold text-primary-foreground">
          C
        </div>
        <div class="min-w-0">
          <h1 class="truncate text-sm font-semibold text-foreground">Capsem Chat</h1>
          <p class="truncate text-xs text-muted-foreground-1">UI authoring lane</p>
        </div>
      </div>
      <div class="flex flex-wrap items-center justify-end gap-2">
        <label class="inline-flex items-center gap-2 text-xs font-medium text-muted-foreground-1">
          <span>Theme</span>
          <select
            class="rounded-lg border border-layer-line bg-layer py-1.5 pe-8 ps-3 text-xs font-medium text-layer-foreground focus:border-primary focus:ring-primary"
            bind:value={selectedTheme}
            onchange={updateTheme}
            aria-label="Theme"
          >
            {#each themes as theme}
              <option value={theme.value}>{theme.label}</option>
            {/each}
          </select>
        </label>
        <label class="inline-flex items-center gap-2 rounded-lg border border-layer-line bg-layer px-3 py-1.5 text-xs font-medium text-layer-foreground">
          <input
            type="checkbox"
            class="rounded border-line-2 bg-surface text-primary focus:ring-primary"
            bind:checked={darkMode}
            onchange={updateTheme}
            aria-label="Dark mode"
          />
          <span>Dark</span>
        </label>
        {#if chatSurfaces.length}
          <span class="inline-flex items-center rounded-full bg-primary px-2.5 py-1 text-xs font-medium text-primary-foreground">
            {chatSurfaces[0].label}
          </span>
        {/if}
      </div>
    </header>

    {#if error}
      <div class="m-4 rounded-lg border border-destructive bg-destructive/10 p-4 text-sm text-destructive">
        {error}
      </div>
    {:else if renderedSurfaces.length === 0}
      <div class="flex flex-1 items-center justify-center text-sm text-muted-foreground-1">
        Loading chat...
      </div>
    {:else}
      <div class="flex-1 space-y-5 overflow-auto bg-surface px-4 py-6">
        <div class="flex justify-end">
          <div class="max-w-[78%] rounded-2xl rounded-tr-sm bg-primary px-4 py-3 text-sm text-primary-foreground shadow-2xs">
            Build me a shell of a chat app and show the generated component inside it.
          </div>
        </div>

        <div class="flex items-start gap-3">
          <div class="flex size-8 shrink-0 items-center justify-center rounded-full border border-card-line bg-card text-xs font-semibold text-foreground">
            AI
          </div>
          <div class="min-w-0 flex-1">
            <div class="mb-2 text-xs font-medium text-muted-foreground-1">assistant</div>
            <div class="rounded-2xl rounded-tl-sm border border-card-line bg-card p-4 shadow-2xs">
              <div class="grid gap-4">
                {#each renderedSurfaces as item (item.surfaceId)}
                  <div class="rounded-xl border border-card-line bg-surface p-4">
                    <A2Node id="root" surface={item.surface} recipe={item.recipe} scope={item.surface.data} onAction={recordAction} />
                  </div>
                {/each}
              </div>
            </div>
          </div>
        </div>

        {#if actions.length}
          <div class="ms-11 grid gap-2">
            {#each actions as action}
              <div class="rounded-lg border border-card-line bg-card px-3 py-2 text-xs text-muted-foreground-1">
                {action.name}
              </div>
            {/each}
          </div>
        {/if}
      </div>

      <footer class="border-t border-card-line bg-card p-4">
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
      </footer>
    {/if}
  </section>
</main>
