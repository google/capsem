<script lang="ts">
  import { onMount } from 'svelte';
  import BracketsAngle from 'phosphor-svelte/lib/BracketsAngle';
  import Briefcase from 'phosphor-svelte/lib/Briefcase';
  import * as api from '../../api';
  import type { ProfileListRecord } from '../../types/gateway';

  type DisplayProfile = {
    id: string;
    name: string;
    description: string;
    bestFor: string;
    ui: string;
  };

  const fallbackProfiles: DisplayProfile[] = [
    {
      id: 'coding',
      name: 'Coding',
      description: 'Focused defaults for software development sessions.',
      bestFor: 'Coding agents, repository work, tests, and developer tooling.',
      ui: 'coding',
    },
    {
      id: 'everyday-work',
      name: 'Everyday Work',
      description: 'Balanced defaults for daily work sessions.',
      bestFor: 'Daily work with useful tools and measured security prompts.',
      ui: 'everyday',
    },
  ];

  let profiles = $state<DisplayProfile[]>(fallbackProfiles);
  let defaultProfile = $state<string | null>('everyday-work');

  onMount(async () => {
    try {
      const response = await api.listProfiles();
      const rows = response.profiles.map(profileFromRecord);
      if (rows.length > 0) profiles = rows;
      defaultProfile = response.default_profile ?? defaultProfile;
    } catch {
      // The service may not be running yet. Keep the screen useful with the
      // built-in profile introduction instead of surfacing internal state.
    }
  });

  function profileFromRecord(record: ProfileListRecord): DisplayProfile {
    return {
      id: record.profile.id,
      name: record.profile.name || record.profile.id,
      description: record.profile.description || 'A ready-to-use Capsem session profile.',
      bestFor: record.profile.best_for || 'General agent work.',
      ui: record.profile.ui || 'everyday',
    };
  }

  function isSelected(profile: DisplayProfile): boolean {
    return profile.id === defaultProfile;
  }
</script>

<div class="text-center space-y-6">
  <div class="size-16 mx-auto rounded-2xl bg-primary/10 flex items-center justify-center">
    <svg class="size-8 text-primary" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
      <path d="M5 13l4 4L19 7" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  </div>

  <div>
    <h2 class="text-xl font-medium text-foreground">You're ready to start</h2>
    <p class="mt-2 text-sm text-muted-foreground-1">
      Start a session with the profile that matches your work. Profiles bundle the tools, model access, security rules, and workspace defaults for that kind of session.
    </p>
  </div>

  <div class="text-left space-y-3">
    {#each profiles as profile (profile.id)}
      <div
        class="bg-card border rounded-xl p-4"
        class:border-primary={isSelected(profile)}
        class:border-card-line={!isSelected(profile)}
      >
        <div class="flex gap-3">
          <div class="size-10 shrink-0 rounded-lg bg-primary/10 text-primary flex items-center justify-center">
            {#if profile.ui === 'coding'}
              <BracketsAngle size={20} />
            {:else}
              <Briefcase size={20} />
            {/if}
          </div>
          <div class="min-w-0">
            <div class="flex items-center gap-2">
              <h3 class="text-sm font-medium text-foreground">{profile.name}</h3>
              {#if isSelected(profile)}
                <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-primary/10 text-primary font-medium">Default</span>
              {/if}
            </div>
            <p class="mt-1 text-xs text-muted-foreground-1">{profile.description}</p>
            <p class="mt-1 text-xs text-muted-foreground">{profile.bestFor}</p>
          </div>
        </div>
      </div>
    {/each}
  </div>

  <div class="rounded-lg border border-card-line bg-card p-3 text-left">
    <p class="text-xs text-muted-foreground-1">
      After this, use <span class="font-medium text-foreground">New Session</span> to choose a profile and launch your workspace.
    </p>
  </div>
</div>
