<script lang="ts">
  import Terminal from '../components/Terminal.svelte';
  import StatsBar from '../components/StatsBar.svelte';
</script>

<div class="flex h-full flex-col bg-black">
  <div class="flex-1 min-h-0">
    <Terminal />
  </div>
  <StatsBar />
</div>
