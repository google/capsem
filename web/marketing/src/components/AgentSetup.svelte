<script lang="ts">
  import { onDestroy, tick } from "svelte";
  import { DOCS, SETUP } from "$lib/data";
  import { setupLink, setupPrompt } from "$lib/agent-setup";
  let { agent, icon }: { agent?: string; icon?: string } = $props();
  const id = $props.id();
  const prompt = $derived(setupPrompt(agent));
  const choices = $derived(agent ? [{ name: agent, icon: icon ?? null }] : SETUP.agents);
  let panel: HTMLDivElement;
  let field: HTMLTextAreaElement;
  let expanded = $state(false);
  let status = $state("");
  let timer: ReturnType<typeof setTimeout> | undefined;
  onDestroy(() => clearTimeout(timer));
  async function copy() {
    try { await navigator.clipboard.writeText(prompt); status = SETUP.copied; }
    catch {
      expanded = true;
      status = SETUP.failed;
      // Wait for Svelte to render the manual fallback before selecting it.
      await tick();
      field.focus();
      field.select();
    }
    clearTimeout(timer);
    timer = setTimeout(() => { status = ""; }, 3000);
  }
</script>

<button type="button" popovertarget={id} aria-label={agent ? `${SETUP.title}: ${agent}` : SETUP.title}
  class={agent ? "group flex w-full items-center gap-2 rounded-xl p-2 text-left text-sm font-semibold sm:gap-3 sm:p-3 sm:text-lg transition-colors hover:bg-accent-soft hover:text-accent-text" : "inline-flex items-center gap-2 rounded-full border border-line bg-paper px-4 py-3 text-sm font-semibold text-ink transition-colors hover:border-accent-border hover:bg-accent-soft hover:text-accent-text"}>
  {#if agent && icon}<img src={`/agents/${icon}.svg`} class="size-7 shrink-0 object-contain" width="28" height="28" alt="" loading="lazy" />{/if}
  <span>{agent ?? SETUP.title}</span>
  <svg class={agent ? "ml-auto size-3.5 shrink-0 text-muted group-hover:text-accent-text" : "size-3.5 shrink-0"} viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><path d="m4 6 4 4 4-4" /></svg>
</button>

<div bind:this={panel} id={id} popover="auto" aria-label={agent ? `${SETUP.title}: ${agent}` : SETUP.title}
  class="agent-setup rounded-2xl border border-line bg-paper p-2 text-ink shadow-xl">
  <p class="px-3 pt-2 pb-1 text-xs font-medium text-muted">{agent ?? SETUP.openIn}</p>
  {#each choices as choice}
    {@const href = setupLink(choice.name)}
    {#if href}
      <a {href} onclick={() => panel.hidePopover()} class="flex items-center gap-3 rounded-xl px-3 py-3 text-sm font-medium hover:bg-accent-soft hover:text-accent-text">
        {#if choice.icon}<img src={`/agents/${choice.icon}.svg`} alt="" width="22" height="22" class="size-5.5 object-contain" />{:else}<svg class="size-5.5" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M5 3v17l5-5 4 6 3-2-4-6h8L5 3Z"/></svg>{/if}
        <span>{choice.name}</span><span class="ml-auto text-muted" aria-hidden="true">↗</span>
      </a>
    {/if}
  {/each}
  {#if agent && !setupLink(agent)}<p class="px-3 py-2 text-sm text-body">{SETUP.paste}</p>{/if}
  <div class="mt-1 border-t border-line pt-1">
    <button type="button" onclick={copy} class="flex w-full items-center gap-3 rounded-xl px-3 py-3 text-left text-sm font-medium hover:bg-accent-soft hover:text-accent-text">
      <svg class="size-5.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><rect x="8" y="8" width="12" height="12" rx="2"/><path d="M15 8V4H4v11h4"/></svg>
      {status === SETUP.copied ? SETUP.copied : SETUP.copy}
    </button>
    <details bind:open={expanded} class="px-3 pb-2">
      <summary class="py-1 text-xs text-muted hover:text-accent-text">{SETUP.show}</summary>
      <textarea bind:this={field} readonly value={prompt} aria-label={SETUP.copy} class="mt-2 h-36 w-full resize-none rounded-lg border border-line bg-tint p-3 text-xs leading-relaxed text-body"></textarea>
    </details>
    <a href={DOCS.start} class="block px-3 py-2 text-xs text-accent-text hover:underline">{SETUP.guide} ↗</a>
  </div>
  <span class="sr-only" role="status">{status}</span>
</div>

<style>
  .agent-setup {
    width: min(20rem, calc(100vw - 2rem));
    max-height: min(36rem, calc(100dvh - 2rem));
    overflow-y: auto;
    margin: .75rem 0 0;
    position-area: bottom span-right;
    position-try-fallbacks: flip-inline, flip-block;
  }
  @supports not (position-area: bottom) {
    .agent-setup { margin: auto; }
  }
</style>
