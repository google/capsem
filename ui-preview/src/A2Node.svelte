<script lang="ts">
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

  export let id: string;
  export let surface: SurfaceModel;
  export let scope: unknown = surface.data;
  export let onAction: (action: RenderAction) => void = () => {};

  let open = false;

  $: component = surface.components.get(id);
  $: childItems = component ? resolveChildren(component.children, surface.data, scope) : [];

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
      return "inline-flex min-h-9 items-center justify-center rounded-md border border-slate-950 bg-slate-950 px-3 py-2 text-sm font-medium text-white hover:bg-slate-800";
    }
    if (variant === "borderless") {
      return "inline-flex min-h-9 items-center justify-center rounded-md px-3 py-2 text-sm font-medium text-slate-700 hover:bg-slate-100";
    }
    return "inline-flex min-h-9 items-center justify-center rounded-md border border-slate-200 bg-white px-3 py-2 text-sm font-medium text-slate-800 hover:bg-slate-50";
  }
</script>

{#if component}
  {#if component.component === "Text"}
    <p class={textClass(component.variant)}>
      {resolveDynamic(component.text, surface.data, scope)}
    </p>
  {:else if component.component === "Icon"}
    <span
      class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-slate-200 bg-slate-50 text-sm font-semibold text-slate-700"
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
        <svelte:self id={child.id} {surface} scope={child.scope} {onAction} />
      {/each}
    </div>
  {:else if component.component === "Column" || component.component === "List"}
    <div class={columnClass()}>
      {#each childItems as child (child.key)}
        <svelte:self id={child.id} {surface} scope={child.scope} {onAction} />
      {/each}
    </div>
  {:else if component.component === "Card"}
    <section class="rounded-lg border border-slate-200 bg-white p-4 shadow-sm">
      <svelte:self id={prop("child")} {surface} {scope} {onAction} />
    </section>
  {:else if component.component === "Button"}
    <button class={buttonClass()} type="button" on:click={runButton}>
      <svelte:self id={prop("child")} {surface} {scope} {onAction} />
    </button>
  {:else if component.component === "Modal"}
    <div>
      <svelte:self
        id={prop("trigger")}
        {surface}
        {scope}
        onAction={(action) => {
          open = true;
          onAction(action);
        }}
      />
      {#if open}
        <div class="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/40 p-4">
          <section class="w-full max-w-md rounded-lg border border-slate-200 bg-white p-5 shadow-xl">
            <div class="mb-4 flex items-center justify-between gap-4">
              <p class="text-sm font-semibold text-slate-900">Ask</p>
              <button
                class="inline-flex h-8 w-8 items-center justify-center rounded-md text-slate-500 hover:bg-slate-100"
                type="button"
                aria-label="Close"
                on:click={() => (open = false)}
              >
                x
              </button>
            </div>
            <svelte:self
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
    <hr class="w-full border-slate-200" />
  {:else}
    <div class="rounded-md border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900">
      Unsupported A2UI Basic component: {component.component}
    </div>
  {/if}
{/if}
