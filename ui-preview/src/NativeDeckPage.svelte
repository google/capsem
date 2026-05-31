<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import {
    loadNativeDeckProof,
    tagNameForArtifact,
    type NativeArtifact,
    type NativeDeckProof,
  } from "./nativeDeck";

  let proof = $state<NativeDeckProof | null>(null);
  let selectedId = $state("");
  let error = $state("");
  let refreshTimer: number | undefined;

  let selected = $derived(
    proof?.artifacts.find((artifact) => artifact.id === selectedId) ?? proof?.artifacts[0] ?? null,
  );
  let deckArtifact = $derived(
    proof?.artifacts.find((artifact) => artifact.kind === "slideDeck") ?? null,
  );

  onMount(() => {
    loadNativeDeckProof()
      .then((body) => {
        applyProof(body);
      })
      .catch((cause: Error) => {
        error = cause.message;
      });
    refreshTimer = window.setInterval(refreshProof, 1500);
  });

  onDestroy(() => {
    if (refreshTimer !== undefined) window.clearInterval(refreshTimer);
  });

  function refreshProof(): void {
    loadNativeDeckProof()
      .then(applyProof)
      .catch((cause: Error) => {
        error = cause.message;
      });
  }

  function applyProof(body: NativeDeckProof): void {
    const previousSelectedId = selectedId;
    proof = body;
    selectedId = body.artifacts.some((artifact) => artifact.id === previousSelectedId)
      ? previousSelectedId
      : body.artifacts[0]?.id ?? "";
    error = "";
  }

  function setSpec(node: HTMLElement & { spec?: NativeArtifact }, artifact: NativeArtifact | null) {
    node.spec = artifact ?? undefined;
    return {
      update(next: NativeArtifact | null) {
        node.spec = next ?? undefined;
      },
    };
  }
</script>

<main class="min-h-screen bg-surface text-foreground">
  <div class="mx-auto grid min-h-screen w-full max-w-7xl grid-cols-1 gap-0 lg:grid-cols-[320px_minmax(0,1fr)]">
    <aside class="border-b border-card-line bg-card lg:border-b-0 lg:border-e">
      <div class="border-b border-card-line px-5 py-4">
        <p class="text-xs font-semibold uppercase text-muted-foreground-1">Capsem native proof</p>
        <h1 class="mt-1 text-lg font-semibold text-foreground">Artifact Deck</h1>
      </div>

      {#if proof}
        <div class="grid grid-cols-2 gap-2 border-b border-card-line p-4 text-xs lg:grid-cols-1">
          <div class="rounded-lg bg-surface p-3">
            <div class="text-muted-foreground-1">SQLite rows</div>
            <div class="mt-1 text-lg font-semibold">{proof.summary.sqliteRows}</div>
          </div>
          <div class="rounded-lg bg-surface p-3">
            <div class="text-muted-foreground-1">Charts</div>
            <div class="mt-1 text-lg font-semibold">{proof.summary.chartCount}</div>
          </div>
        </div>
        <nav class="max-h-[calc(100vh-170px)] overflow-auto p-3" aria-label="Native artifacts">
          {#each proof.artifacts as artifact}
            <button
              type="button"
              class={artifact.id === selected?.id
                ? "mb-1 w-full rounded-lg bg-primary px-3 py-2 text-left text-sm font-medium text-primary-foreground"
                : "mb-1 w-full rounded-lg px-3 py-2 text-left text-sm font-medium text-muted-foreground-1 hover:bg-surface-hover hover:text-foreground"}
              onclick={() => (selectedId = artifact.id)}
            >
              <span class="block truncate">{artifact.title}</span>
              <span class="mt-0.5 block text-xs opacity-80">{artifact.kind}</span>
            </button>
          {/each}
        </nav>
      {/if}
    </aside>

    <section class="min-w-0">
      <header class="border-b border-card-line bg-card px-5 py-4">
        <div class="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
          <div>
            <p class="text-xs font-semibold uppercase text-muted-foreground-1">SQLite -> artifacts -> deck</p>
            <h2 class="mt-1 text-xl font-semibold text-foreground">{proof?.title ?? "Loading deck proof"}</h2>
          </div>
          {#if proof}
            <span class="inline-flex w-fit rounded-full bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground">
              {proof.summary.individualArtifactCount} artifacts
            </span>
          {/if}
        </div>
      </header>

      {#if error}
        <div class="m-5 rounded-lg border border-destructive bg-destructive/10 p-4 text-sm text-destructive">
          {error}
        </div>
      {:else if !proof || !selected}
        <div class="m-5 rounded-lg border border-card-line bg-card p-4 text-sm text-muted-foreground-1">
          Loading native artifact proof...
        </div>
      {:else}
        <div class="grid gap-5 p-5">
          <article class="rounded-xl border border-card-line bg-card p-4 shadow-2xs">
            <div class="mb-3 flex items-center justify-between gap-3">
              <h3 class="text-sm font-semibold text-foreground">Selected artifact</h3>
              <span class="rounded-full bg-surface px-2.5 py-1 text-xs font-medium text-muted-foreground-1">
                {tagNameForArtifact(selected)}
              </span>
            </div>
            <svelte:element this={tagNameForArtifact(selected)} use:setSpec={selected}></svelte:element>
          </article>

          {#if deckArtifact}
            <article class="rounded-xl border border-card-line bg-card p-4 shadow-2xs">
              <div class="mb-3 flex items-center justify-between gap-3">
                <h3 class="text-sm font-semibold text-foreground">Deck preview</h3>
                <span class="rounded-full bg-surface px-2.5 py-1 text-xs font-medium text-muted-foreground-1">
                  &lt;capsem-slide-deck&gt;
                </span>
              </div>
              <capsem-slide-deck use:setSpec={deckArtifact}></capsem-slide-deck>
            </article>
          {/if}
        </div>
      {/if}
    </section>
  </div>
</main>
