<script lang="ts">
  import { sidebarStore } from '../stores/sidebar.svelte';
  import { settingsStore } from '../stores/settings.svelte';
</script>

<div class="flex h-full items-center justify-center">
  <div class="text-center max-w-md space-y-4">
    <h1 class="text-2xl font-semibold">Welcome to Capsem</h1>
    <p class="text-sm text-base-content/60">Get started by configuring an AI provider with an API key. You can always change this later in Settings.</p>
    <div class="flex justify-center gap-3 pt-2">
      <button
        class="btn bg-interactive text-white btn-sm"
        onclick={() => {
          const first = settingsStore.sections[0] || 'AI Providers';
          sidebarStore.setSettingsSection(first);
          sidebarStore.setView('settings');
        }}
      >
        Configure Providers
      </button>
      <button
        class="btn btn-ghost btn-sm"
        onclick={() => sidebarStore.setView('terminal')}
      >
        Skip for now
      </button>
    </div>
  </div>
</div>
