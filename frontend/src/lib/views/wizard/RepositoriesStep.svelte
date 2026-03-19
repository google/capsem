<script lang="ts">
  import { wizardStore } from '../../stores/wizard.svelte';
  import { settingsStore } from '../../stores/settings.svelte';
  import { openUrl } from '../../api';
  import KeyValidationBadge from '../../components/KeyValidationBadge.svelte';

  function getLeaf(id: string) {
    return settingsStore.findLeaf(id);
  }

  async function updateSetting(id: string, value: string | boolean) {
    await settingsStore.update(id, value);
  }

  async function clearDetected(id: string) {
    await wizardStore.clearDetected(id);
    await settingsStore.load();
  }

  const repoProviders = [
    { key: 'github', name: 'GitHub', allowId: 'repository.providers.github.allow', tokenId: 'repository.providers.github.token' },
    { key: 'gitlab', name: 'GitLab', allowId: 'repository.providers.gitlab.allow', tokenId: 'repository.providers.gitlab.token' },
  ];

  let showTokens = $state<Record<string, boolean>>({});
</script>

<div class="space-y-6">
  <div>
    <h2 class="text-2xl font-semibold">Repositories</h2>
    <p class="text-sm text-base-content/60 mt-1">
      Configure git identity, SSH keys, and repository provider credentials.
    </p>
  </div>

  <!-- Git Identity -->
  <div class="card border border-base-300 p-4 space-y-3">
    <h3 class="font-semibold">Git Identity</h3>
    <div class="grid grid-cols-2 gap-3">
      <div class="space-y-1">
        <label class="text-xs text-base-content/50" for="git-name">Author name</label>
        <input
          id="git-name"
          type="text"
          class="input input-sm input-bordered w-full text-xs"
          placeholder="Your Name"
          value={String(getLeaf('repository.git.identity.author_name')?.effective_value ?? '')}
          onchange={(e) => updateSetting('repository.git.identity.author_name', (e.target as HTMLInputElement).value)}
        />
        {#if wizardStore.wasAutoApplied('repository.git.identity.author_name')}
          <div class="flex items-center gap-2">
            <span class="text-xs text-allowed">From ~/.gitconfig</span>
            <button class="text-xs text-base-content/40 hover:underline" onclick={() => clearDetected('repository.git.identity.author_name')}>Clear</button>
          </div>
        {/if}
      </div>
      <div class="space-y-1">
        <label class="text-xs text-base-content/50" for="git-email">Author email</label>
        <input
          id="git-email"
          type="email"
          class="input input-sm input-bordered w-full text-xs"
          placeholder="you@example.com"
          value={String(getLeaf('repository.git.identity.author_email')?.effective_value ?? '')}
          onchange={(e) => updateSetting('repository.git.identity.author_email', (e.target as HTMLInputElement).value)}
        />
        {#if wizardStore.wasAutoApplied('repository.git.identity.author_email')}
          <div class="flex items-center gap-2">
            <span class="text-xs text-allowed">From ~/.gitconfig</span>
            <button class="text-xs text-base-content/40 hover:underline" onclick={() => clearDetected('repository.git.identity.author_email')}>Clear</button>
          </div>
        {/if}
      </div>
    </div>
  </div>

  <!-- SSH Public Key -->
  <div class="card border border-base-300 p-4 space-y-3">
    <h3 class="font-semibold">SSH Public Key</h3>
    <textarea
      class="textarea textarea-bordered w-full font-mono text-xs h-16"
      placeholder="ssh-ed25519 AAAA... (optional)"
      value={String(getLeaf('vm.environment.ssh.public_key')?.effective_value ?? '')}
      onchange={(e) => updateSetting('vm.environment.ssh.public_key', (e.target as HTMLTextAreaElement).value)}
    ></textarea>
    <div class="flex items-center gap-2">
      {#if wizardStore.wasAutoApplied('vm.environment.ssh.public_key')}
        <span class="text-xs text-allowed">From ~/.ssh</span>
        <button class="text-xs text-base-content/40 hover:underline" onclick={() => clearDetected('vm.environment.ssh.public_key')}>Clear</button>
      {:else}
        <p class="text-xs text-base-content/40">Injected as /root/.ssh/authorized_keys in the VM.</p>
      {/if}
    </div>
  </div>

  <!-- Repository Providers -->
  {#each repoProviders as prov}
    {@const allowLeaf = getLeaf(prov.allowId)}
    {@const tokenLeaf = getLeaf(prov.tokenId)}
    {@const enabled = allowLeaf?.effective_value === true}
    {@const currentToken = String(tokenLeaf?.effective_value ?? '')}
    {@const detected = wizardStore.wasAutoApplied(prov.tokenId)}

    <div class="card border border-base-300 p-4 space-y-3">
      <div class="flex items-center justify-between">
        <h3 class="font-semibold">{prov.name}</h3>
        <input
          type="checkbox"
          class="toggle toggle-sm"
          checked={enabled}
          onchange={(e) => updateSetting(prov.allowId, (e.target as HTMLInputElement).checked)}
        />
      </div>
      <div class="space-y-1">
        <div class="relative">
          <input
            type={showTokens[prov.tokenId] ? 'text' : 'password'}
            class="input input-sm input-bordered w-full pr-16 font-mono text-xs"
            placeholder={tokenLeaf?.metadata?.prefix ? `${tokenLeaf.metadata.prefix}...` : 'Token'}
            value={currentToken}
            onchange={(e) => updateSetting(prov.tokenId, (e.target as HTMLInputElement).value)}
          />
          <button
            class="btn btn-ghost btn-xs absolute right-1 top-1/2 -translate-y-1/2 text-base-content/40"
            onclick={() => showTokens[prov.tokenId] = !showTokens[prov.tokenId]}
          >
            {showTokens[prov.tokenId] ? 'Hide' : 'Show'}
          </button>
        </div>
        <div class="flex items-center gap-2">
          {#if detected && currentToken}
            <span class="text-xs text-allowed">Detected via gh CLI</span>
            <button class="text-xs text-base-content/40 hover:underline" onclick={() => clearDetected(prov.tokenId)}>Clear</button>
          {/if}
          {#if prov.key === 'github' && currentToken}
            <KeyValidationBadge provider="github" apiKey={currentToken} />
          {/if}
          {#if tokenLeaf?.metadata?.docs_url}
            <button
              class="text-xs text-base-content/40 hover:underline"
              onclick={() => openUrl(tokenLeaf!.metadata.docs_url!)}
            >
              Get a token
            </button>
          {/if}
        </div>
      </div>
    </div>
  {/each}

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
