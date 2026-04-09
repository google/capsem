<script lang="ts">
  import { tabStore } from '../../stores/tabs.svelte.ts';
  import { themeStore, PRELINE_THEMES } from '../../stores/theme.svelte.ts';
  import ArrowClockwise from 'phosphor-svelte/lib/ArrowClockwise';
  import Stop from 'phosphor-svelte/lib/Stop';
  import Trash from 'phosphor-svelte/lib/Trash';
  import GitFork from 'phosphor-svelte/lib/GitFork';
  import MagnifyingGlass from 'phosphor-svelte/lib/MagnifyingGlass';
  import Sun from 'phosphor-svelte/lib/Sun';
  import Moon from 'phosphor-svelte/lib/Moon';
  import Desktop from 'phosphor-svelte/lib/Desktop';
  import List from 'phosphor-svelte/lib/List';
  import Palette from 'phosphor-svelte/lib/Palette';
  import Info from 'phosphor-svelte/lib/Info';
  import GearSix from 'phosphor-svelte/lib/GearSix';

  let active = $derived(tabStore.active);
  let isVM = $derived(active?.vmId != null);
  let menuOpen = $state(false);

  function onClickOutside(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (!target.closest('[data-menu]')) {
      menuOpen = false;
    }
  }
</script>

<svelte:document onclick={onClickOutside} />

<div class="flex items-center gap-x-2 bg-layer border-b border-line-2 px-2 py-1">
  <!-- Left: VM actions -->
  <div class="flex items-center gap-x-0.5">
    <button
      type="button"
      class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover disabled:opacity-30 disabled:pointer-events-none"
      disabled={!isVM}
      aria-label="Restart"
      title="Restart VM"
    >
      <ArrowClockwise size={16} />
    </button>
    <button
      type="button"
      class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover disabled:opacity-30 disabled:pointer-events-none"
      disabled={!isVM}
      aria-label="Stop"
      title="Stop VM"
    >
      <Stop size={16} />
    </button>
    <button
      type="button"
      class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-destructive hover:bg-muted-hover disabled:opacity-30 disabled:pointer-events-none"
      disabled={!isVM}
      aria-label="Destroy"
      title="Destroy VM"
    >
      <Trash size={16} />
    </button>
    <button
      type="button"
      class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover disabled:opacity-30 disabled:pointer-events-none"
      disabled={!isVM}
      aria-label="Fork"
      title="Fork VM"
    >
      <GitFork size={16} />
    </button>
  </div>

  <!-- Center: search / address bar -->
  <div class="flex-1 flex items-center gap-x-2 bg-background-1 rounded-lg px-3 py-1 min-w-0">
    <MagnifyingGlass size={14} class="shrink-0 text-muted-foreground" />
    {#if active}
      <span class="text-xs text-muted-foreground-1 shrink-0">{active.view}</span>
      <span class="text-xs text-muted-foreground">/</span>
      <span class="text-xs text-foreground truncate">{active.title}</span>
    {/if}
  </div>

  <!-- Right: menu button -->
  <div class="relative" data-menu>
    <button
      type="button"
      class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover"
      onclick={(e: MouseEvent) => { e.stopPropagation(); menuOpen = !menuOpen; }}
      aria-label="Menu"
      title="Menu"
    >
      <List size={16} />
    </button>

    {#if menuOpen}
      <div class="absolute end-0 top-full mt-1 w-56 bg-dropdown border border-dropdown-border rounded-xl shadow-lg z-50">
        <div class="p-1">
          <!-- Mode toggle (auto -> light -> dark -> auto) -->
          <button
            type="button"
            class="w-full flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover"
            onclick={() => themeStore.toggleMode()}
          >
            {#if themeStore.modePref === 'auto'}
              <Desktop size={16} />
              <span>Auto ({themeStore.mode})</span>
            {:else if themeStore.modePref === 'dark'}
              <Moon size={16} />
              <span>Dark</span>
            {:else}
              <Sun size={16} />
              <span>Light</span>
            {/if}
          </button>

          <div class="border-t border-dropdown-divider my-1"></div>

          <!-- Preline theme select -->
          <div class="flex items-center gap-x-3 py-2 px-3">
            <Palette size={16} class="shrink-0 text-dropdown-item-foreground" />
            <select
              class="flex-1 text-sm rounded-lg border border-line-2 bg-layer text-foreground py-1 px-2 focus:outline-hidden focus:border-primary"
              value={themeStore.prelineTheme}
              onchange={(e) => themeStore.setPrelineTheme((e.target as HTMLSelectElement).value)}
            >
              {#each PRELINE_THEMES as theme (theme.value)}
                <option value={theme.value}>{theme.label}</option>
              {/each}
            </select>
          </div>

          <div class="border-t border-dropdown-divider my-1"></div>

          <!-- Other menu items -->
          <button
            type="button"
            class="w-full flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover"
            onclick={() => { tabStore.add('settings', 'Settings'); menuOpen = false; }}
          >
            <GearSix size={16} />
            <span>Settings</span>
          </button>
          <button
            type="button"
            class="w-full flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover"
            onclick={() => { tabStore.add('settings', 'About'); menuOpen = false; }}
          >
            <Info size={16} />
            <span>About Capsem</span>
          </button>
        </div>
      </div>
    {/if}
  </div>
</div>
