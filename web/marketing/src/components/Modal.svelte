<script lang="ts">
  import type { Snippet } from "svelte";
  let { open = $bindable(false), title, closeLabel, children }: { open?: boolean; title: string; closeLabel: string; children: Snippet } = $props();
  const titleId = $props.id();
  let dialog: HTMLDialogElement;
  function close() { open = false; dialog.close(); }
  function outside(event: PointerEvent) {
    if (event.target !== dialog) return;
    const rect = dialog.getBoundingClientRect();
    if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) close();
  }
  $effect(() => {
    if (!open) { dialog?.close(); return; }
    const previous = document.activeElement;
    const overflow = document.documentElement.style.overflow;
    dialog.showModal();
    document.documentElement.style.overflow = "hidden";
    return () => {
      dialog.close();
      document.documentElement.style.overflow = overflow;
      if (previous instanceof HTMLElement) previous.focus();
    };
  });
</script>

<dialog bind:this={dialog} aria-labelledby={titleId} onpointerdown={outside}
  oncancel={(event) => { event.preventDefault(); close(); }}
  onkeydown={(event) => { if (event.key === "Escape") { event.preventDefault(); close(); } }}
  onclose={() => { open = false; }}
  class="m-auto max-h-[calc(100dvh_-_3rem)] w-[calc(100%_-_3rem)] max-w-md overflow-y-auto rounded-2xl border border-line bg-paper p-6 text-ink shadow-2xl backdrop:bg-ink/40 backdrop:backdrop-blur-sm sm:p-8">
  <header class="mb-5 flex items-center justify-between gap-4">
    <h2 id={titleId} class="text-2xl font-semibold tracking-tight">{title}</h2>
    <button type="button" class="grid size-10 shrink-0 place-items-center rounded-full border border-line bg-tint text-muted transition-colors hover:border-accent-border hover:bg-accent-soft hover:text-accent-text" aria-label={closeLabel} onclick={close}>
      <svg class="size-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" aria-hidden="true"><path d="m6 6 12 12M6 18 18 6"/></svg>
    </button>
  </header>
  {@render children()}
</dialog>
