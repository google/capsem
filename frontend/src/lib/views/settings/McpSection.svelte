<script lang="ts">
  import { settingsStore } from '../../stores/settings.svelte';
  import type { ResolvedSetting, SettingValue } from '../../types';

  function handleChange(setting: ResolvedSetting, value: SettingValue) {
    settingsStore.update(setting.id, value);
  }

  function inputType(st: string): string {
    switch (st) {
      case 'password':
      case 'apikey':
        return 'password';
      case 'number':
        return 'number';
      default:
        return 'text';
    }
  }

  // MCP settings are not yet in the settings registry, so show a placeholder.
  // When MCP server config settings are added to the backend, they'll appear here.
</script>

<div class="space-y-4">
  <div class="text-sm text-base-content/40 py-8 text-center">
    MCP server configuration coming soon.
  </div>
  <div class="text-[11px] text-base-content/40 text-center">
    Configure MCP servers, test connectivity, list available tools, and set per-tool allow/warn/block policies.
  </div>
</div>
