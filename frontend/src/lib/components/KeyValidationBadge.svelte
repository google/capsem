<script lang="ts">
  import { validateApiKey } from '../api';
  import type { KeyValidation } from '../types';

  interface Props {
    provider: string;
    apiKey: string;
  }

  let { provider, apiKey }: Props = $props();

  type ValidationPhase = 'idle' | 'validating' | 'done';
  let phase: ValidationPhase = $state('idle');
  let result: KeyValidation | null = $state(null);
  let debounceTimer: ReturnType<typeof setTimeout> | undefined;

  $effect(() => {
    const key = apiKey;
    const prov = provider;

    clearTimeout(debounceTimer);
    phase = 'idle';
    result = null;

    if (!key || !key.trim()) return;

    debounceTimer = setTimeout(async () => {
      phase = 'validating';
      try {
        result = await validateApiKey(prov, key);
      } catch {
        result = { valid: false, message: 'Validation failed' };
      }
      phase = 'done';
    }, 600);

    return () => clearTimeout(debounceTimer);
  });
</script>

{#if phase === 'validating'}
  <span class="inline-flex items-center gap-1 text-xs text-base-content/50">
    <span class="loading loading-spinner loading-xs"></span>
    Checking...
  </span>
{:else if phase === 'done' && result}
  {#if result.valid}
    <span class="text-xs text-allowed">{result.message}</span>
  {:else}
    <span class="text-xs text-denied">{result.message}</span>
  {/if}
{/if}
