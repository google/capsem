<script lang="ts">
  import { onMount } from 'svelte';
  import * as api from '../../api';
  import type { SettingsNode, SettingsLeaf } from '../../types/settings';
  import type { DetectedConfigSummary } from '../../types/onboarding';

  let loading = $state(true);
  let validating = $state<string | null>(null);
  let validationResults = $state<Record<string, { valid: boolean; message: string }>>({});
  let keyInputs = $state<Record<string, string>>({});

  type ProviderDef = {
    id: string;       // validate_api_key provider name
    name: string;     // display name
    settingId: string; // setting leaf ID
    credentialId: string; // Profile V2 service credential ID
    detectedKey: keyof Pick<
      DetectedConfigSummary,
      | 'anthropic_api_key_present'
      | 'openai_api_key_present'
      | 'google_api_key_present'
      | 'github_token_present'
    >;
  };

  type ProviderInfo = ProviderDef & {
    configured: boolean;
    corpLocked: boolean;
    docsUrl: string | null; // where to get a key
  };

  const providerDefs: ProviderDef[] = [
    {
      id: 'anthropic',
      name: 'Anthropic',
      settingId: 'ai.anthropic.api_key',
      credentialId: 'anthropic-api-key',
      detectedKey: 'anthropic_api_key_present',
    },
    {
      id: 'openai',
      name: 'OpenAI',
      settingId: 'ai.openai.api_key',
      credentialId: 'openai-api-key',
      detectedKey: 'openai_api_key_present',
    },
    {
      id: 'google',
      name: 'Google AI',
      settingId: 'ai.google.api_key',
      credentialId: 'google-api-key',
      detectedKey: 'google_api_key_present',
    },
    {
      id: 'github',
      name: 'GitHub',
      settingId: 'repository.providers.github.token',
      credentialId: 'github-token',
      detectedKey: 'github_token_present',
    },
  ];

  function fallbackProviders(): ProviderInfo[] {
    return providerDefs.map(p => ({
      ...p,
      configured: false,
      corpLocked: false,
      docsUrl: null,
    }));
  }

  let providers = $state<ProviderInfo[]>(fallbackProviders());
  let gitName = $state<string | null>(null);
  let gitEmail = $state<string | null>(null);
  let sshConfigured = $state(false);
  let oauthConfigured = $state(false);

  function findLeaf(tree: SettingsNode[], id: string): SettingsLeaf | null {
    for (const node of tree) {
      if (node.kind === 'leaf' && node.id === id) return node;
      if ('children' in node && node.children) {
        const found = findLeaf(node.children as SettingsNode[], id);
        if (found) return found;
      }
    }
    return null;
  }

  function isPopulated(leaf: SettingsLeaf | null): boolean {
    if (!leaf) return false;
    const v = leaf.effective_value;
    if (typeof v === 'string') return v.length > 0;
    if (typeof v === 'object' && v !== null && 'content' in v) return v.content.length > 0;
    return false;
  }

  function credentialIdsFromSettings(settings: Awaited<ReturnType<typeof api.getSettings>>): Set<string> {
    const ids = settings.settings_profiles?.service?.credential_ids;
    return new Set(Array.isArray(ids) ? ids : []);
  }

  function detectedProviderPresent(summary: DetectedConfigSummary | null, p: (typeof providerDefs)[number]): boolean {
    return Boolean(summary?.[p.detectedKey]);
  }

  function ensureKeyInputs() {
    for (const p of providers) {
      if (!p.configured && !p.corpLocked && keyInputs[p.id] === undefined) {
        keyInputs[p.id] = '';
      }
    }
  }

  onMount(async () => {
    let detected: DetectedConfigSummary | null = null;
    try {
      detected = await api.runDetection();
    } catch { /* */ }

    try {
      const settings = await api.getSettings();
      const tree = Array.isArray(settings.tree) ? settings.tree : [];
      const credentialIds = credentialIdsFromSettings(settings);

      providers = providerDefs.map(p => {
        const leaf = findLeaf(tree, p.settingId);
        return {
          ...p,
          configured:
            credentialIds.has(p.credentialId) || isPopulated(leaf) || detectedProviderPresent(detected, p),
          corpLocked: leaf?.corp_locked ?? false,
          docsUrl: leaf?.metadata?.docs_url ?? null,
        };
      });

      // Git identity
      const nameLeaf = findLeaf(tree, 'repository.git.identity.author_name');
      const emailLeaf = findLeaf(tree, 'repository.git.identity.author_email');
      gitName = (nameLeaf?.effective_value as string) || null;
      gitEmail = (emailLeaf?.effective_value as string) || null;
      gitName = gitName || detected?.git_name || null;
      gitEmail = gitEmail || detected?.git_email || null;

      // SSH
      const sshLeaf = findLeaf(tree, 'vm.environment.ssh.public_key');
      sshConfigured = isPopulated(sshLeaf) || Boolean(detected?.ssh_public_key_present);

      // Claude OAuth
      const oauthLeaf = findLeaf(tree, 'ai.anthropic.claude.credentials_json');
      oauthConfigured = isPopulated(oauthLeaf) || Boolean(detected?.claude_oauth_present);
    } catch {
      providers = providerDefs.map(p => ({
        ...p,
        configured: detectedProviderPresent(detected, p),
        corpLocked: false,
        docsUrl: null,
      }));
      gitName = detected?.git_name ?? null;
      gitEmail = detected?.git_email ?? null;
      sshConfigured = Boolean(detected?.ssh_public_key_present);
      oauthConfigured = Boolean(detected?.claude_oauth_present);
    } finally {
      ensureKeyInputs();
      loading = false;
    }
  });

  async function validateAndSave(p: ProviderInfo) {
    const key = keyInputs[p.id]?.trim();
    if (!key) return;

    validating = p.id;
    try {
      const result = await api.validateApiKey(p.id, key);
      validationResults[p.id] = result;

      if (result.valid) {
        await api.saveCredential(p.credentialId, key, `${p.name} API key`);
        // Mark as configured
        const idx = providers.findIndex(x => x.id === p.id);
        if (idx >= 0) providers[idx].configured = true;
      }
    } catch {
      validationResults[p.id] = { valid: false, message: 'Validation failed' };
    } finally {
      validating = null;
    }
  }
</script>

<div class="space-y-6">
  <div>
    <h2 class="text-xl font-medium text-foreground">AI Providers</h2>
    <p class="mt-1 text-sm text-muted-foreground-1">
      Review detected credentials. Add any missing keys below.
    </p>
  </div>

  {#if loading}
    <div class="flex items-center gap-2 text-sm text-muted-foreground-1">
      <svg class="size-4 animate-spin" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M12 2v4m0 12v4m-7.07-3.93l2.83-2.83m8.48-8.48l2.83-2.83M2 12h4m12 0h4M4.93 4.93l2.83 2.83m8.48 8.48l2.83 2.83" stroke-linecap="round" />
      </svg>
      Loading settings...
    </div>
  {:else}
    <div class="space-y-3">
      {#each providers as p}
        <div class="bg-card border border-card-line rounded-xl p-4">
          <div class="flex items-center justify-between">
            <span class="text-sm font-medium text-foreground">{p.name}</span>
            {#if p.corpLocked}
              <span class="text-xs text-muted-foreground">Corp managed</span>
            {:else if p.configured || validationResults[p.id]?.valid}
              <span class="flex items-center gap-1 text-xs text-primary">
                <svg class="size-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
                  <path d="M5 13l4 4L19 7" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
                Configured
              </span>
            {/if}
          </div>

          {#if !p.configured && !p.corpLocked && !validationResults[p.id]?.valid}
            <div class="flex gap-2 mt-2">
              <input
                type="password"
                bind:value={keyInputs[p.id]}
                placeholder="Enter API key..."
                class="flex-1 py-1.5 px-3 text-sm border border-line-2 rounded-lg bg-layer text-foreground placeholder:text-muted-foreground focus:border-primary focus:ring-1 focus:ring-primary outline-none"
                disabled={validating === p.id}
              />
              <button
                type="button"
                class="py-1.5 px-3 text-xs font-medium rounded-lg bg-layer border border-layer-line text-layer-foreground hover:bg-layer-hover transition-colors disabled:opacity-50"
                disabled={!keyInputs[p.id]?.trim() || validating === p.id}
                onclick={() => validateAndSave(p)}
              >
                {validating === p.id ? 'Checking...' : 'Validate'}
              </button>
            </div>
            {#if p.docsUrl}
              <a
                href={p.docsUrl}
                target="_blank"
                rel="noopener noreferrer"
                class="mt-1 inline-block text-xs text-primary hover:underline"
              >Get a key &rarr;</a>
            {/if}
            {#if validationResults[p.id] && !validationResults[p.id].valid}
              <p class="mt-1 text-xs text-destructive">{validationResults[p.id].message}</p>
            {/if}
          {/if}
        </div>
      {/each}
    </div>

    <div class="text-xs text-muted-foreground space-y-1">
      {#if gitName}
        <p>Git identity: {gitName}{#if gitEmail} &lt;{gitEmail}&gt;{/if}</p>
      {/if}
      {#if sshConfigured}
        <p>SSH key configured</p>
      {/if}
      {#if oauthConfigured}
        <p>Claude OAuth credentials configured</p>
      {/if}
    </div>
  {/if}
</div>
