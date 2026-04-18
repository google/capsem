<script lang="ts">
  import X from 'phosphor-svelte/lib/X';

  let {
    open = false,
    title = '',
    confirmLabel = 'Confirm',
    cancelLabel = 'Cancel',
    destructive = false,
    disabled = false,
    onconfirm,
    oncancel,
    children,
  }: {
    open: boolean;
    title: string;
    confirmLabel?: string;
    cancelLabel?: string;
    destructive?: boolean;
    disabled?: boolean;
    onconfirm: () => void;
    oncancel: () => void;
    children?: any;
  } = $props();

  function handleBackdrop() {
    oncancel();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === 'Escape') oncancel();
    if (e.key === 'Enter' && !disabled) onconfirm();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if open}
  <!-- Backdrop -->
  <button
    type="button"
    class="fixed inset-0 z-80 bg-black/50 transition-opacity cursor-default border-none p-0 m-0"
    onclick={handleBackdrop}
    aria-label="Close dialog"
  ></button>

  <!-- Dialog -->
  <div class="fixed inset-0 z-80 overflow-y-auto flex items-center justify-center p-4">
    <div
      class="bg-overlay border border-overlay-border shadow-2xs rounded-xl sm:max-w-md w-full"
      role="dialog"
      aria-modal="true"
      aria-label={title}
    >
      <!-- Header -->
      <div class="flex justify-between items-center py-3 px-4 border-b border-overlay-border">
        <h3 class="font-bold text-foreground">{title}</h3>
        <button
          type="button"
          class="size-8 inline-flex justify-center items-center rounded-full bg-muted text-muted-foreground-1 hover:bg-muted-hover transition-colors"
          onclick={oncancel}
        >
          <X size={16} />
        </button>
      </div>

      <!-- Body -->
      <div class="p-4">
        {@render children?.()}
      </div>

      <!-- Footer -->
      <div class="flex justify-end items-center gap-x-2 py-3 px-4 border-t border-overlay-border">
        <button
          type="button"
          class="py-2 px-3 text-sm font-medium rounded-lg border border-layer-line bg-layer text-layer-foreground hover:bg-layer-hover transition-colors"
          onclick={oncancel}
        >
          {cancelLabel}
        </button>
        <button
          type="button"
          class="py-2 px-3 text-sm font-medium rounded-lg transition-colors
            {destructive
              ? 'bg-destructive text-destructive-foreground hover:bg-destructive/90'
              : 'bg-primary text-primary-foreground hover:bg-primary-hover'}
            {disabled ? 'opacity-50 cursor-not-allowed' : ''}"
          onclick={onconfirm}
          {disabled}
        >
          {confirmLabel}
        </button>
      </div>
    </div>
  </div>
{/if}
