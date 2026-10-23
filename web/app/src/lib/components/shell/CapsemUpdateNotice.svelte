<script lang="ts">
  import { onMount } from 'svelte';
  import { getUpdateStatus } from '../../api';
  import { capsemUpdateVersion } from '../../models/capsem-update';
  import { watchVisibleRefresh } from '../../models/visible-refresh';
  import { tabStore } from '../../stores/tabs.svelte';

  let version = $state<string | null>(null);

  onMount(() => {
    let active = true;
    let pending = false;
    const refresh = async () => {
      if (pending) return;
      pending = true;
      try {
        const status = await getUpdateStatus();
        if (active) version = capsemUpdateVersion(status);
      } catch {
        if (active) version = null;
      } finally {
        pending = false;
      }
    };
    void refresh();
    const stop = watchVisibleRefresh(refresh, 30_000);
    return () => { active = false; stop(); };
  });
</script>

{#if version}
  <button type="button" class="shrink-0 rounded-md px-2 py-1 text-xs text-primary hover:bg-muted-hover"
    title="Review the available Capsem update" onclick={() => tabStore.openSingleton('about', 'About Capsem')}>
    Capsem {version} available
  </button>
{/if}
