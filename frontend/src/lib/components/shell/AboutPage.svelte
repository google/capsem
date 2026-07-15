<script lang="ts">
  import { onMount } from 'svelte';
  import * as api from '../../api';
  import type { CapsemStatus } from '../../api';

  type JsonObject = Record<string, unknown>;

  let system = $state<CapsemStatus | null>(null);
  let error = $state<string | null>(null);
  let refreshing = $state(false);
  let copied = $state(false);
  let requestGeneration = 0;

  onMount(() => { void refreshStatus(); });

  function object(value: unknown): JsonObject {
    return value && typeof value === 'object' && !Array.isArray(value) ? value as JsonObject : {};
  }

  function array(value: unknown): unknown[] {
    return Array.isArray(value) ? value : [];
  }

  function text(value: unknown, fallback = 'unknown'): string {
    return typeof value === 'string' && value.length > 0 ? value : fallback;
  }

  function manifestProfiles(): Array<{ id: string; profile: JsonObject }> {
    return Object.entries(object(system?.manifest.profiles)).map(([id, profile]) => ({
      id,
      profile: object(profile),
    }));
  }

  function liveProfile(id: string): JsonObject {
    return object(array(system?.profiles.profiles).find((entry) => object(entry).id === id));
  }

  function profileArchitectures(profile: JsonObject): JsonObject[] {
    return array(profile.architectures).map(object);
  }

  function selectedArchitecture(id: string, profile: JsonObject): JsonObject {
    const architectures = profileArchitectures(profile);
    const current = liveProfile(id).current_arch;
    return architectures.find((arch) => arch.architecture === current) ?? architectures[0] ?? {};
  }

  function profileEvidence(id: string, profile: JsonObject): JsonObject[] {
    return array(selectedArchitecture(id, profile).evidence).map(object);
  }

  function packageRecords(): JsonObject[] {
    const packages = array(system?.manifest.packages).map(object);
    const platform = typeof navigator !== 'undefined' && /Mac/i.test(navigator.userAgent)
      ? 'macos'
      : 'linux';
    const platformPackages = packages.filter((pkg) => pkg.platform === platform);
    const exact = platformPackages.filter((pkg) => pkg.version === system?.version);
    if (exact.length > 0) return exact;

    const currentArchitecture = manifestProfiles()
      .map(({ id }) => liveProfile(id).current_arch)
      .find((value) => typeof value === 'string');
    const channelArchitecture = platformPackages.filter((pkg) => pkg.architecture === currentArchitecture);
    return channelArchitecture.length > 0 ? channelArchitecture : platformPackages;
  }

  function packageMatchesInstalled(pkg: JsonObject): boolean {
    return pkg.version === system?.version;
  }

  function packageEvidence(pkg: JsonObject): JsonObject[] {
    return array(pkg.evidence).map(object);
  }

  function packageBinaries(pkg: JsonObject): JsonObject[] {
    return array(pkg.binaries).map(object);
  }

  function resolveManifestUrl(value: unknown): string {
    const url = text(value, '');
    if (!url) return '';
    try {
      return new URL(url, text(system?.manifest_metadata.manifest_url, '')).toString();
    } catch {
      return url;
    }
  }

  function timestamp(value: unknown): string {
    if (typeof value === 'number') return new Date(value * 1000).toLocaleString();
    if (typeof value === 'string' && value) return value;
    return 'not recorded';
  }

  async function refreshStatus() {
    const generation = ++requestGeneration;
    error = null;
    try {
      const next = await api.getCapsemStatus();
      if (generation === requestGeneration) system = next;
    } catch (err) {
      if (generation === requestGeneration) error = err instanceof Error ? err.message : String(err);
    }
  }

  async function checkForUpdates() {
    const generation = ++requestGeneration;
    refreshing = true;
    error = null;
    try {
      await api.checkForUpdates();
      const next = await api.getCapsemStatus();
      if (generation === requestGeneration) system = next;
    } catch (err) {
      if (generation === requestGeneration) error = err instanceof Error ? err.message : String(err);
    } finally {
      if (generation === requestGeneration) refreshing = false;
    }
  }

  async function copyStatus() {
    if (!system) return;
    await navigator.clipboard.writeText(JSON.stringify(system, null, 2));
    copied = true;
    window.setTimeout(() => { copied = false; }, 1500);
  }
</script>

<main class="h-full overflow-y-auto bg-background">
  <div class="mx-auto max-w-5xl px-8 py-8">
    <div class="flex items-start justify-between gap-x-6 mb-8">
      <div>
        <h1 class="text-2xl font-bold text-foreground">About Capsem</h1>
        <p class="text-sm text-muted-foreground-1 mt-1">Installed manifest, profiles, binaries, and update state.</p>
      </div>
      <div class="flex gap-x-2">
        <button type="button" class="py-2 px-4 text-sm font-medium rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover disabled:opacity-60" disabled={!system} onclick={copyStatus}>
          {copied ? 'Copied' : 'Copy status'}
        </button>
        <button type="button" class="py-2 px-4 text-sm font-medium rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover disabled:opacity-60" disabled={refreshing} onclick={checkForUpdates}>
          {refreshing ? 'Refreshing' : 'Refresh'}
        </button>
      </div>
    </div>

    {#if error}
      <div class="mb-6 rounded-xl border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm text-destructive">{error}</div>
    {/if}

    {#if !system}
      <div class="flex items-center justify-center py-24"><div class="animate-spin size-6 border-2 border-primary border-t-transparent rounded-full"></div></div>
    {:else}
      <h2 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">Installed system</h2>
      <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider mb-6">
        <div class="flex items-center justify-between p-4"><span class="text-sm text-foreground">Capsem</span><span class="text-sm text-muted-foreground-1">{system.version}</span></div>
        <div class="flex items-center justify-between p-4"><span class="text-sm text-foreground">Service</span><span class="text-sm text-muted-foreground-1">{system.service}</span></div>
        <div class="flex items-center justify-between p-4"><span class="text-sm text-foreground">Channel</span><span class="text-sm text-muted-foreground-1">{text(system.manifest.channel)}</span></div>
        <div class="flex items-center justify-between p-4"><span class="text-sm text-foreground">Manifest version</span><span class="text-sm text-muted-foreground-1">{text(system.manifest.version)}</span></div>
        <div class="flex items-center justify-between p-4">
          <span class="text-sm text-foreground">Update</span>
          <span class="text-sm {system.updates.binary.update_available ? 'text-primary' : 'text-muted-foreground-1'}">
            {system.updates.binary.update_available ? `${system.updates.binary.latest} available` : 'Current'}
          </span>
        </div>
      </div>

      <h2 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">VM profiles</h2>
      <div class="grid gap-4 mb-6 sm:grid-cols-2">
        {#each manifestProfiles() as { id, profile } (id)}
          {@const live = liveProfile(id)}
          {@const architecture = selectedArchitecture(id, profile)}
          <section class="bg-card border border-card-line rounded-xl p-4">
            <div class="flex items-start justify-between gap-4">
              <div>
                <h3 class="text-base font-semibold text-foreground">{text(profile.name, id)}</h3>
                <p class="text-sm text-muted-foreground-1 mt-1">{text(profile.description, '')}</p>
              </div>
              <span class="text-xs {live.ready === true ? 'text-primary' : 'text-destructive'}">{live.ready === true ? 'Ready' : 'Not ready'}</span>
            </div>
            <dl class="grid grid-cols-[7rem_1fr] gap-x-3 gap-y-1 mt-4 text-xs">
              <dt class="text-muted-foreground-1">Revision</dt><dd class="text-foreground break-all">{text(profile.revision)}</dd>
              <dt class="text-muted-foreground-1">Architecture</dt><dd class="text-foreground">{text(architecture.architecture)}</dd>
              <dt class="text-muted-foreground-1">Profile hash</dt><dd class="text-foreground break-all">{text(live.profile_payload_hash)}</dd>
            </dl>
            {#if profileEvidence(id, profile).length > 0}
              <div class="flex flex-wrap gap-3 mt-4">
                {#each profileEvidence(id, profile) as evidence}
                  {#if resolveManifestUrl(evidence.url)}
                    <a class="text-xs font-medium text-primary hover:underline" href={resolveManifestUrl(evidence.url)} target="_blank" rel="noreferrer">{text(evidence.kind).toUpperCase()}</a>
                  {/if}
                {/each}
              </div>
            {/if}
          </section>
        {/each}
      </div>

      <h2 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">Installed package</h2>
      <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider mb-6">
        {#each packageRecords() as pkg (text(pkg.name))}
          <div class="p-4">
            {#if !packageMatchesInstalled(pkg)}
              <p class="mb-3 rounded-lg border border-warning/30 bg-warning/5 px-3 py-2 text-xs text-warning">
                Channel package differs from installed Capsem: package {text(pkg.version)}, installed {system.version}.
              </p>
            {/if}
            <div class="flex items-start justify-between gap-4">
              <div><p class="text-sm font-medium text-foreground">{text(pkg.name)}</p><p class="text-xs text-muted-foreground-1 mt-1">{text(pkg.platform)} · {text(pkg.architecture)} · {text(pkg.version)}</p></div>
              <span class="text-xs text-muted-foreground-1">{text(pkg.status)}</span>
            </div>
            <div class="flex flex-wrap gap-3 mt-3">
              {#each packageEvidence(pkg) as evidence}
                {#if resolveManifestUrl(evidence.url)}
                  <a class="text-xs font-medium text-primary hover:underline" href={resolveManifestUrl(evidence.url)} target="_blank" rel="noreferrer">Host {text(evidence.kind).toUpperCase()}</a>
                {/if}
              {/each}
            </div>
            <details class="mt-4">
              <summary class="cursor-pointer text-sm font-medium text-foreground">{packageMatchesInstalled(pkg) ? 'Installed package binaries' : 'Channel package binaries'} ({packageBinaries(pkg).length})</summary>
              <div class="mt-3 divide-y divide-card-divider border-t border-card-divider">
                {#each packageBinaries(pkg) as binary (text(binary.installed_path, text(binary.name)))}
                  <div class="py-3">
                    <div class="flex items-center justify-between gap-4"><span class="text-sm text-foreground">{text(binary.name)}</span><span class="text-xs text-muted-foreground-1">{text(binary.version, text(pkg.version))}</span></div>
                    <p class="text-xs text-muted-foreground-1 mt-1 break-all">{text(binary.installed_path)}</p>
                    <p class="text-xs text-muted-foreground-1 mt-1 break-all">SHA-256 {text(object(binary.digest).sha256, text(binary.sha256))}</p>
                  </div>
                {/each}
              </div>
            </details>
          </div>
        {:else}
          <div class="p-4 text-sm text-destructive">Installed manifest contains no package records for this platform.</div>
        {/each}
      </div>

      <h2 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">Manifest metadata</h2>
      <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
        <div class="p-4"><p class="text-sm text-foreground">Manifest URL</p><p class="text-xs text-muted-foreground-1 mt-1 break-all">{text(system.manifest_metadata.manifest_url)}</p></div>
        <div class="flex items-center justify-between p-4"><span class="text-sm text-foreground">Installed</span><span class="text-sm text-muted-foreground-1">{timestamp(system.manifest_metadata.installed_at ?? system.manifest_metadata.packaged_at)}</span></div>
        <div class="flex items-center justify-between p-4"><span class="text-sm text-foreground">Refreshed</span><span class="text-sm text-muted-foreground-1">{timestamp(system.manifest_metadata.refreshed_at)}</span></div>
        <div class="flex items-center justify-between p-4"><span class="text-sm text-foreground">Last checked</span><span class="text-sm text-muted-foreground-1">{timestamp(system.manifest_metadata.checked_at)}</span></div>
        <div class="p-4"><p class="text-sm text-foreground">Checked URL</p><p class="text-xs text-muted-foreground-1 mt-1 break-all">{text(system.manifest_metadata.checked_url)}</p></div>
        <div class="flex items-center justify-between p-4"><span class="text-sm text-foreground">Validation</span><span class="text-sm {system.manifest_metadata.validation_status === 'valid' ? 'text-primary' : 'text-destructive'}">{text(system.manifest_metadata.validation_status)}</span></div>
        {#if system.manifest_metadata.validation_error}
          <div class="p-4 text-xs text-destructive">{String(system.manifest_metadata.validation_error)}</div>
        {/if}
      </div>
    {/if}
  </div>
</main>
