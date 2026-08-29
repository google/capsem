<script lang="ts">
  import { onMount } from 'svelte';
  import { vmStore } from '../../stores/vms.svelte.ts';
  import { tabStore } from '../../stores/tabs.svelte.ts';
  import { listProfiles, type ProfileSummary } from '../../api';
  import Modal from './Modal.svelte';

  let profiles = $state<ProfileSummary[]>([]);
  let profileId = $state('');
  let name = $state('');
  let ramMb = $state(2048);
  let cpus = $state(2);
  let error = $state<string | null>(null);
  let creating = $state(false);

  onMount(async () => {
    try {
      profiles = (await listProfiles()).profiles.filter(profile => profile.availability.web);
      profileId = vmStore.createProfileId ?? profiles[0]?.id ?? '';
    } catch {
      profiles = [];
      profileId = '';
      error = 'Could not load installed profiles';
    }
  });

  function close() {
    vmStore.closeCreateModal();
    profileId = vmStore.createProfileId ?? profiles[0]?.id ?? '';
    name = '';
    ramMb = 2048;
    cpus = 2;
    error = null;
  }

  async function handleSubmit() {
    error = null;
    const trimmedName = name.trim();
    if (!trimmedName) {
      error = 'Name is required';
      return;
    }
    if (!profileId) {
      error = 'Could not load installed profiles';
      return;
    }
    creating = true;
    try {
      const { id, name: finalName } = await vmStore.provision({
        profile_id: profileId,
        name: trimmedName,
        ram_mb: ramMb,
        cpus: cpus,
        persistent: true,
      });
      tabStore.openVM(id, finalName);
      close();
    } catch (e: any) {
      error = e.message || 'Failed to create sandbox';
    } finally {
      creating = false;
    }
  }

  $effect(() => {
    if (vmStore.showCreateModal && vmStore.createProfileId) {
      profileId = vmStore.createProfileId;
    }
  });
</script>

<Modal
  open={vmStore.showCreateModal}
  title="Customize session"
  confirmLabel={creating ? 'Creating...' : 'Create'}
  onconfirm={handleSubmit}
  oncancel={close}
  disabled={creating || !profileId}
>
  <div class="space-y-4 py-2">
    {#if error}
      <div class="p-3 rounded-lg bg-destructive/10 border border-destructive/20 text-destructive text-sm">
        {error}
      </div>
    {/if}

    <div class="space-y-1.5">
      <label for="sb-profile" class="text-sm font-medium text-foreground">Profile</label>
      <select
        id="sb-profile"
        bind:value={profileId}
        class="w-full px-3 py-2 rounded-lg bg-background-1 border border-line-2 focus:border-primary outline-hidden text-sm text-foreground"
        disabled={creating}
      >
        {#if profiles.length === 0}
          <option value="">No installed profiles available</option>
        {:else}
          {#each profiles as profile (profile.id)}
            <option value={profile.id}>{profile.name}</option>
          {/each}
        {/if}
      </select>
      <p class="text-[11px] text-muted-foreground-1">
        {profiles.find(profile => profile.id === profileId)?.description ?? 'Profile-selected VM configuration.'}
      </p>
    </div>

    <div class="space-y-1.5">
      <label for="sb-name" class="text-sm font-medium text-foreground">Name</label>
      <input
        id="sb-name"
        type="text"
        bind:value={name}
        placeholder="coding-agent"
        class="w-full px-3 py-2 rounded-lg bg-background-1 border border-line-2 focus:border-primary focus:ring-2 focus:ring-primary/20 outline-hidden transition-all text-sm text-foreground"
        disabled={creating}
      />
      <p class="text-[11px] text-muted-foreground-1">Each VM is named and tied to its selected profile.</p>
    </div>

    <div class="grid grid-cols-2 gap-4">
      <div class="space-y-1.5">
        <label for="sb-ram" class="text-sm font-medium text-foreground">RAM (MB)</label>
        <select
          id="sb-ram"
          bind:value={ramMb}
          class="w-full px-3 py-2 rounded-lg bg-background-1 border border-line-2 focus:border-primary outline-hidden text-sm text-foreground"
          disabled={creating}
        >
          <option value={1024}>1024 MB (1 GB)</option>
          <option value={2048}>2048 MB (2 GB)</option>
          <option value={4096}>4096 MB (4 GB)</option>
          <option value={8192}>8192 MB (8 GB)</option>
        </select>
      </div>

      <div class="space-y-1.5">
        <label for="sb-cpus" class="text-sm font-medium text-foreground">CPUs</label>
        <select
          id="sb-cpus"
          bind:value={cpus}
          class="w-full px-3 py-2 rounded-lg bg-background-1 border border-line-2 focus:border-primary outline-hidden text-sm text-foreground"
          disabled={creating}
        >
          <option value={1}>1 CPU</option>
          <option value={2}>2 CPUs</option>
          <option value={4}>4 CPUs</option>
          <option value={8}>8 CPUs</option>
        </select>
      </div>
    </div>
  </div>
</Modal>
