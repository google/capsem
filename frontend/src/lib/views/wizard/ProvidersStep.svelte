<script lang="ts">
  import { wizardStore } from '../../stores/wizard.svelte';
  import { settingsStore } from '../../stores/settings.svelte';
  import { openUrl } from '../../api';

  const providers = [
    { key: 'anthropic', name: 'Anthropic', allowId: 'ai.anthropic.allow', keyId: 'ai.anthropic.api_key' },
    { key: 'google', name: 'Google AI', allowId: 'ai.google.allow', keyId: 'ai.google.api_key' },
    { key: 'openai', name: 'OpenAI', allowId: 'ai.openai.allow', keyId: 'ai.openai.api_key' },
  ];

  let showKeys = $state<Record<string, boolean>>({});

  function getLeaf(id: string) {
    return settingsStore.findLeaf(id);
  }

  async function toggleProvider(allowId: string, enabled: boolean) {
    await settingsStore.update(allowId, enabled);
  }

  async function updateKey(keyId: string, value: string) {
    await settingsStore.update(keyId, value);
  }

  async function clearKey(keyId: string) {
    await wizardStore.clearDetected(keyId);
    await settingsStore.load();
  }
</script>

<div class="space-y-6">
  <div>
    <h2 class="text-2xl font-semibold">AI Providers</h2>
    <p class="text-sm text-base-content/60 mt-1">
      Enable at least one provider and add an API key to get started.
    </p>
  </div>

  <div class="space-y-4">
    {#each providers as prov}
      {@const allowLeaf = getLeaf(prov.allowId)}
      {@const keyLeaf = getLeaf(prov.keyId)}
      {@const enabled = allowLeaf?.effective_value === true}
      {@const currentKey = String(keyLeaf?.effective_value ?? '')}
      {@const corpLocked = allowLeaf?.corp_locked || false}
      {@const detected = wizardStore.wasAutoApplied(prov.keyId)}

      <div class="card border border-base-300 p-4 space-y-3">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-2">
            <h3 class="font-semibold">{prov.name}</h3>
            {#if corpLocked}
              <span class="badge badge-sm text-xs text-base-content/40">Corp locked</span>
            {/if}
          </div>
          <input
            type="checkbox"
            class="toggle toggle-sm"
            checked={enabled}
            disabled={corpLocked}
            onchange={(e) => toggleProvider(prov.allowId, (e.target as HTMLInputElement).checked)}
          />
        </div>

        <div class="space-y-1">
          <div class="flex items-center gap-2">
            <div class="relative flex-1">
              <input
                type={showKeys[prov.keyId] ? 'text' : 'password'}
                class="input input-sm input-bordered w-full pr-16 font-mono text-xs"
                placeholder={keyLeaf?.metadata?.prefix ? `${keyLeaf.metadata.prefix}...` : 'API key'}
                value={currentKey}
                disabled={corpLocked}
                onchange={(e) => updateKey(prov.keyId, (e.target as HTMLInputElement).value)}
              />
              <button
                class="btn btn-ghost btn-xs absolute right-1 top-1/2 -translate-y-1/2 text-base-content/40"
                onclick={() => showKeys[prov.keyId] = !showKeys[prov.keyId]}
              >
                {showKeys[prov.keyId] ? 'Hide' : 'Show'}
              </button>
            </div>
          </div>
          <div class="flex items-center gap-2">
            {#if detected && currentKey}
              <span class="text-xs text-allowed">Detected on your system</span>
              <button
                class="text-xs text-base-content/40 hover:underline"
                onclick={() => clearKey(prov.keyId)}
              >
                Clear
              </button>
            {/if}
            {#if keyLeaf?.metadata?.docs_url}
              <button
                class="text-xs text-base-content/40 hover:underline"
                onclick={() => openUrl(keyLeaf!.metadata.docs_url!)}
              >
                Get a key
              </button>
            {/if}
          </div>
        </div>
      </div>
    {/each}
  </div>

  <!-- Nav -->
  <div class="flex justify-between pt-4">
    <button class="btn btn-ghost btn-sm" onclick={() => wizardStore.back()}>Back</button>
    <div class="flex gap-2">
      <button class="btn btn-ghost btn-sm" onclick={() => wizardStore.next()}>Skip</button>
      <button class="btn bg-interactive text-white btn-sm" onclick={() => wizardStore.next()}>
        Next
      </button>
    </div>
  </div>
</div>
