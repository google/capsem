<script lang="ts">
  import A2Node from "./A2Node.svelte";
  import {
    actionContext,
    actionName,
    childComponent,
    componentById,
    iconText,
    resolveChildren,
    resolveDynamic,
    staticChildIds,
    textClass,
    textValue,
    type RenderAction,
    type PreviewRecipe,
    type SurfaceModel,
  } from "./a2ui";

  let {
    id,
    surface,
    recipe,
    scope = surface.data,
    tone = "default",
    onAction = () => {},
  }: {
    id: string;
    surface: SurfaceModel;
    recipe?: PreviewRecipe;
    scope?: unknown;
    tone?: string;
    onAction?: (action: RenderAction) => void;
  } = $props();

  let open = $state(false);

  let component = $derived(surface.components.get(id));
  let childItems = $derived(
    component ? resolveChildren(component.children, surface.data, scope) : [],
  );
  let cardChild = $derived(component?.component === "Card" ? componentById(surface, prop("child")) : undefined);
  let modalContent = $derived(component?.component === "Modal" ? componentById(surface, prop("content")) : undefined);

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
      return "py-3 px-4 inline-flex justify-center items-center gap-x-2 text-sm font-medium rounded-lg bg-primary border border-primary-line text-primary-foreground hover:bg-primary-hover focus:outline-hidden focus:bg-primary-focus disabled:opacity-50 disabled:pointer-events-none";
    }
    if (variant === "borderless") {
      return "py-3 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg border border-transparent text-primary hover:text-primary-hover focus:outline-hidden focus:text-primary-hover disabled:opacity-50 disabled:pointer-events-none";
    }
    return "py-3 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg bg-layer border border-layer-line text-layer-foreground shadow-2xs hover:bg-layer-hover focus:outline-hidden focus:bg-layer-focus disabled:opacity-50 disabled:pointer-events-none";
  }

  function isAlertCard(): boolean {
    return component?.component === "Card" && recipe?.component === "alert";
  }

  function isStatusCard(): boolean {
    return component?.component === "Card" && recipe?.variant === "top-border";
  }

  function isSimpleCard(): boolean {
    return component?.component === "Card" && recipe?.component === "card" && recipe?.variant === "simple";
  }

  function isTableCard(): boolean {
    return component?.component === "Card" && recipe?.component === "table" && recipe?.variant === "basic";
  }

  function isBasicModal(): boolean {
    return component?.component === "Modal" && recipe?.component === "modal";
  }

  function tableChildComponents(): Array<NonNullable<typeof component>> {
    if (!cardChild) return [];
    return staticChildIds(cardChild)
      .map((childId) => surface.components.get(childId))
      .filter((child): child is NonNullable<typeof component> => Boolean(child));
  }

  function tableTitle(): string {
    const title = tableChildComponents().find((child) => child.component === "Text");
    return title ? textValue(surface, title, scope) : "";
  }

  function tableRows(): Array<NonNullable<typeof component>> {
    return tableChildComponents().filter((child) => child.component === "Row");
  }

  function tableCells(row: NonNullable<typeof component>): Array<NonNullable<typeof component>> {
    return staticChildIds(row)
      .map((childId) => surface.components.get(childId))
      .filter((child): child is NonNullable<typeof component> => Boolean(child));
  }

  function alertClass(): string {
    if (recipe?.tone === "warning") {
      return "bg-warning/10 border border-warning/20 rounded-lg shadow-2xs p-4";
    }
    return "bg-layer border border-layer-line rounded-lg shadow-lg p-4";
  }

  function iconName(): unknown {
    return component?.name;
  }

  function renderableIconName(): string {
    return iconText(iconName());
  }
</script>

{#if component}
  {#if component.component === "Text"}
    <p class={textClass(component.variant, tone)}>
      {resolveDynamic(component.text, surface.data, scope)}
    </p>
  {:else if component.component === "Icon"}
    {#if renderableIconName() === "!"}
      <svg class="shrink-0 size-4 text-primary mt-1" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M12 9v4"></path>
        <path d="M12 17h.01"></path>
        <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z"></path>
      </svg>
    {:else}
      <svg class="shrink-0 size-4 text-primary mt-1" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="10"></circle>
        <path d="M12 16v-4"></path>
        <path d="M12 8h.01"></path>
      </svg>
    {/if}
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
        <A2Node id={child.id} {surface} {recipe} scope={child.scope} {onAction} />
      {/each}
    </div>
  {:else if component.component === "Column" || component.component === "List"}
    <div class={columnClass()}>
      {#each childItems as child (child.key)}
        <A2Node id={child.id} {surface} {recipe} scope={child.scope} {onAction} />
      {/each}
    </div>
  {:else if component.component === "Card" && isAlertCard()}
    {@const icon = childComponent(surface, cardChild, 0)}
    {@const message = childComponent(surface, cardChild, 1)}
    <div
      class={alertClass()}
      role="alert"
      tabindex="-1"
      aria-labelledby={`${surface.surfaceId}-alert-label`}
    >
      <div class="flex">
        <div class="shrink-0">
          {#if icon}
            <svg class="shrink-0 size-4 text-primary mt-1" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10"></circle>
              <path d="M12 16v-4"></path>
              <path d="M12 8h.01"></path>
            </svg>
          {/if}
        </div>
        <div class="ms-3">
          <h3 id={`${surface.surfaceId}-alert-label`} class="text-foreground font-semibold">
            Security notice
          </h3>
          {#if message}
            <A2Node id={message.id} {surface} {recipe} {scope} tone="alert" {onAction} />
          {/if}
        </div>
      </div>
    </div>
  {:else if component.component === "Card" && isStatusCard()}
    {@const statusRow = childComponent(surface, cardChild, 0)}
    {@const statusMessage = childComponent(surface, cardChild, 1)}
    {@const statusIcon = childComponent(surface, statusRow, 0)}
    {@const statusTitle = childComponent(surface, statusRow, 1)}
    <div class="bg-card border-t-2 border-primary rounded-lg p-4 shadow-2xs" role="status">
      <div class="flex">
        <div class="shrink-0">
          {#if statusIcon}
            <A2Node id={statusIcon.id} {surface} {recipe} {scope} {onAction} />
          {/if}
        </div>
        <div class="ms-3">
          {#if statusTitle}
            <h3 class="font-semibold text-foreground">
              {textValue(surface, statusTitle, scope)}
            </h3>
          {/if}
          {#if statusMessage}
            <p class="mt-1 text-sm text-muted-foreground-1">
              {textValue(surface, statusMessage, scope)}
            </p>
          {/if}
        </div>
      </div>
    </div>
  {:else if component.component === "Card" && isSimpleCard()}
    {@const title = childComponent(surface, cardChild, 0)}
    {@const description = childComponent(surface, cardChild, 1)}
    {@const meta = childComponent(surface, cardChild, 2)}
    <div class="flex flex-col bg-card border border-card-line shadow-2xs rounded-xl">
      <div class="p-4">
        {#if title}
          <h3 class="font-semibold text-foreground">
            {textValue(surface, title, scope)}
          </h3>
        {/if}
        {#if meta}
          {@const metaCaption = childComponent(surface, meta, 1)}
          {#if metaCaption}
            <p class="mt-1 text-xs font-medium uppercase text-muted-foreground-1">
              {textValue(surface, metaCaption, scope)}
            </p>
          {/if}
        {/if}
        {#if description}
          <p class="mt-1 text-sm text-muted-foreground-1">
            {textValue(surface, description, scope)}
          </p>
        {/if}
        <a class="mt-3 inline-flex items-center gap-x-1 text-sm font-semibold rounded-lg border border-transparent text-primary decoration-2 hover:text-primary-hover hover:underline focus:underline focus:outline-hidden focus:text-primary-focus disabled:opacity-50 disabled:pointer-events-none" href={recipe?.link?.href ?? recipe?.docsUrl ?? "#"}>
          {recipe?.link?.label ?? "Card link"}
          <svg class="shrink-0 size-4" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m9 18 6-6-6-6"></path>
          </svg>
        </a>
      </div>
    </div>
  {:else if component.component === "Card" && isTableCard()}
    {@const title = tableTitle()}
    {@const rows = tableRows()}
    {@const header = rows[0]}
    {@const bodyRows = rows.slice(1)}
    <div class="flex flex-col bg-card border border-card-line shadow-2xs rounded-xl overflow-hidden">
      {#if title}
        <div class="px-6 py-4 border-b border-card-line">
          <h3 class="font-semibold text-foreground">{title}</h3>
        </div>
      {/if}
      <div class="overflow-x-auto">
        <table class="min-w-full divide-y divide-card-line">
          {#if header}
            <thead class="bg-surface">
              <tr>
                {#each tableCells(header) as cell (cell.id)}
                  <th scope="col" class="px-6 py-3 text-start text-xs font-semibold uppercase text-muted-foreground-1">
                    {textValue(surface, cell, scope)}
                  </th>
                {/each}
              </tr>
            </thead>
          {/if}
          <tbody class="divide-y divide-card-line">
            {#each bodyRows as row (row.id)}
              <tr class="bg-card hover:bg-surface">
                {#each tableCells(row) as cell, cellIndex (cell.id)}
                  {#if cellIndex === 0}
                    <th scope="row" class="px-6 py-4 whitespace-nowrap text-sm font-medium text-foreground">
                      {textValue(surface, cell, scope)}
                    </th>
                  {:else}
                    <td class="px-6 py-4 whitespace-nowrap text-sm text-muted-foreground-1">
                      {textValue(surface, cell, scope)}
                    </td>
                  {/if}
                {/each}
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </div>
  {:else if component.component === "Card"}
    <div class="flex flex-col bg-card border border-card-line shadow-2xs rounded-xl">
      <div class="p-4">
        <A2Node id={prop("child")} {surface} {recipe} {scope} {onAction} />
      </div>
    </div>
  {:else if component.component === "Button"}
    <button class={buttonClass()} type="button" onclick={runButton}>
      <A2Node id={prop("child")} {surface} {scope} {onAction} />
    </button>
  {:else if component.component === "Modal" && isBasicModal()}
    <div>
      <A2Node
        id={prop("trigger")}
        {surface}
        {recipe}
        {scope}
        onAction={(action) => {
          open = true;
          onAction(action);
        }}
      />
      {#if open}
        <div class="fixed inset-0 z-50 overflow-y-auto bg-overlay/80 backdrop-blur-xs">
          <div class="flex min-h-full items-center justify-center p-4">
            <section class="w-full max-w-lg flex flex-col bg-overlay border border-overlay-line shadow-2xs rounded-xl">
              <div class="flex justify-between items-center py-3 px-4 border-b border-overlay-header">
                <h3 class="font-semibold text-foreground">Question</h3>
                <button
                  class="size-8 inline-flex justify-center items-center gap-x-2 rounded-full bg-surface border border-surface-line text-surface-foreground hover:bg-surface-hover focus:outline-hidden focus:bg-surface-focus disabled:opacity-50 disabled:pointer-events-none"
                  type="button"
                  aria-label="Close"
                  onclick={() => (open = false)}
                >
                  <span class="sr-only">Close</span>
                  <svg class="shrink-0 size-4" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M18 6 6 18"></path>
                    <path d="m6 6 12 12"></path>
                  </svg>
                </button>
              </div>
              <div class="overflow-y-auto p-4">
                {#if modalContent}
                  {@const message = childComponent(surface, modalContent, 0)}
                  {@const actions = childComponent(surface, modalContent, 1)}
                  {#if message}
                    <p class="text-sm text-muted-foreground-1">
                      {textValue(surface, message, scope)}
                    </p>
                  {/if}
                  {#if actions}
                    <div class="mt-4 flex justify-end gap-x-2">
                      {#each staticChildIds(actions) as actionId}
                        <A2Node
                          id={actionId}
                          {surface}
                          {recipe}
                          {scope}
                          onAction={(action) => {
                            open = false;
                            onAction(action);
                          }}
                        />
                      {/each}
                    </div>
                  {/if}
                {/if}
              </div>
            </section>
          </div>
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
