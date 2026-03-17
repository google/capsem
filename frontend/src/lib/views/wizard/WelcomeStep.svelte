<script lang="ts">
  import { wizardStore } from '../../stores/wizard.svelte';
</script>

<div class="flex flex-col items-center text-center space-y-6 pt-12">
  <h1 class="text-3xl font-semibold">Capsem</h1>
  <p class="text-lg text-base-content/60">Sandboxed AI agents on your Mac</p>
  <p class="text-sm text-base-content/50 max-w-md">
    Capsem runs AI coding agents inside an air-gapped Linux VM with full network control, telemetry,
    and tool sandboxing. This wizard will help you configure security, providers, and credentials.
  </p>
  <p class="text-xs text-base-content/40">
    The VM image is downloading in the background while you set things up.
  </p>
  <button
    class="btn bg-interactive text-white mt-4"
    onclick={() => wizardStore.next()}
  >
    Get Started
  </button>
</div>
