<script lang="ts">
  import A2Node from "./A2Node.svelte";
  import {
    actionContext,
    actionName,
    iconText,
    resolveChildren,
    resolveDynamic,
    textClass,
    type RenderAction,
    type SurfaceModel,
  } from "./a2ui";

  let {
    id,
    surface,
    scope = surface.data,
    onAction = () => {},
  }: {
    id: string;
    surface: SurfaceModel;
    scope?: unknown;
    onAction?: (action: RenderAction) => void;
  } = $props();

  let open = $state(false);

  let component = $derived(surface.components.get(id));
  let childItems = $derived(
    component ? resolveChildren(component.children, surface.data, scope) : [],
  );

  function prop(name: string): string {
    const value = component?.[name];
    return typeof value === "string" ? value : "";
  }

  function runButton(): void {
    if (!component) return;
    onAction({
      name: actionName(component),
      context: actionContext(component),
      sourceComponentId: component.id,
    });
  }

  function rowClass(): string {
    const align = component?.align === "center" ? "items-center" : "items-start";
    const justify = component?.justify === "spaceAround"
      ? "justify-around"
      : component?.justify === "spaceBetween"
        ? "justify-between"
        : "justify-start";
    return `flex gap-3 ${align} ${justify}`;
  }

  function columnClass(): string {
    const align = component?.align === "center" ? "items-center text-center" : "items-start";
    return `flex flex-col gap-3 ${align}`;
  }

  function buttonClass(): string {
    const variant = component?.variant;
    if (variant === "primary") {
      return "inline-flex min-h-9 items-center justify-center rounded-lg border border-primary bg-primary px-3 py-2 text-sm font-medium text-primary-foreground hover:bg-primary-hover";
    }
    if (variant === "borderless") {
      return "inline-flex min-h-9 items-center justify-center rounded-lg px-3 py-2 text-sm font-medium text-layer-foreground hover:bg-layer-hover";
    }
    return "inline-flex min-h-9 items-center justify-center rounded-lg border border-layer-line bg-layer px-3 py-2 text-sm font-medium text-layer-foreground hover:bg-layer-hover";
  }
</script>

{#if component}
  {#if component.component === "Text"}
    <p class={textClass(component.variant)}>
      {resolveDynamic(component.text, surface.data, scope)}
    </p>
  {:else if component.component === "Icon"}
    <span
      class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-layer-line bg-surface text-sm font-semibold text-muted-foreground-1"
      aria-hidden="true"
    >
      {iconText(component.name)}
    </span>
  {:else if component.component === "Image"}
    <img
      class={component.variant === "avatar"
        ? "h-10 w-10 rounded-full object-cover"
        : "max-h-56 w-full rounded-md object-cover"}
      src={resolveDynamic(component.url, surface.data, scope)}
      alt={resolveDynamic(component.description, surface.data, scope)}
    />
  {:else if component.component === "Row"}
    <div class={rowClass()}>
      {#each childItems as child (child.key)}
        <A2Node id={child.id} {surface} scope={child.scope} {onAction} />
      {/each}
    </div>
  {:else if component.component === "Column" || component.component === "List"}
    <div class={columnClass()}>
      {#each childItems as child (child.key)}
        <A2Node id={child.id} {surface} scope={child.scope} {onAction} />
      {/each}
    </div>
  {:else if component.component === "Card"}
    <section class="flex flex-col bg-card border border-card-line shadow-2xs rounded-xl">
      <div class="p-4 md:p-5">
        <A2Node id={prop("child")} {surface} {scope} {onAction} />
      </div>
    </section>
  {:else if component.component === "Button"}
    <button class={buttonClass()} type="button" onclick={runButton}>
      <A2Node id={prop("child")} {surface} {scope} {onAction} />
    </button>
  {:else if component.component === "Modal"}
    <div>
      <A2Node
        id={prop("trigger")}
        {surface}
        {scope}
        onAction={(action) => {
          open = true;
          onAction(action);
        }}
      />
      {#if open}
        <div class="fixed inset-0 z-50 flex items-center justify-center bg-foreground/40 p-4">
          <section class="w-full max-w-md rounded-xl border border-overlay-line bg-overlay p-5 shadow-xl">
            <div class="mb-4 flex items-center justify-between gap-4">
              <p class="text-sm font-semibold text-foreground">Ask</p>
              <button
                class="inline-flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground-1 hover:bg-layer-hover"
                type="button"
                aria-label="Close"
                onclick={() => (open = false)}
              >
                x
              </button>
            </div>
            <A2Node
              id={prop("content")}
              {surface}
              {scope}
              onAction={(action) => {
                open = false;
                onAction(action);
              }}
            />
          </section>
        </div>
      {/if}
    </div>
  {:else if component.component === "Divider"}
    <hr class="w-full border-card-divider" />
  {:else}
    <div class="rounded-lg border border-card-line bg-surface p-3 text-sm text-destructive">
      Unsupported A2UI Basic component: {component.component}
    </div>
  {/if}
{/if}
