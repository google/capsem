// Wizard store -- tracks setup wizard step and host detection state.
import { detectHostConfig, saveSettings } from '../api';
import type { HostConfig } from '../types';

const STEP_IDS = ['welcome', 'security', 'providers', 'repositories', 'mcp', 'allset'] as const;
export type WizardStepId = (typeof STEP_IDS)[number];

class WizardStore {
  currentStep = $state(0);
  completed = $state(false);
  hostConfig = $state<HostConfig | null>(null);
  hostConfigLoading = $state(false);
  /** Track which settings were auto-applied from host detection. */
  autoApplied = $state<Set<string>>(new Set());

  stepId = $derived(STEP_IDS[this.currentStep] as WizardStepId);
  isFirstStep = $derived(this.currentStep === 0);
  isLastStep = $derived(this.currentStep === STEP_IDS.length - 1);
  canGoBack = $derived(this.currentStep > 0);
  totalSteps = STEP_IDS.length;

  next() {
    if (this.currentStep < STEP_IDS.length - 1) {
      this.currentStep++;
    }
  }

  back() {
    if (this.currentStep > 0) {
      this.currentStep--;
    }
  }

  finish() {
    this.completed = true;
    this.forceRerun = false;
  }

  /** Set by the "Re-run Setup Wizard" button in Settings. */
  forceRerun = $state(false);

  /** Reset wizard state so it can be re-run from Settings.
   *  Clears hostConfig so loadHostConfig() re-detects SSH keys, API keys,
   *  and GH tokens. Clears autoApplied so stale "Detected" badges
   *  don't appear. */
  rerun() {
    this.currentStep = 0;
    this.completed = false;
    this.forceRerun = true;
    this.hostConfig = null;
    this.autoApplied = new Set();
  }

  async loadHostConfig() {
    if (this.hostConfig || this.hostConfigLoading) return;
    this.hostConfigLoading = true;
    try {
      this.hostConfig = await detectHostConfig();
      await this.applyDetected(this.hostConfig);
    } catch (e) {
      console.error('Failed to detect host config:', e);
      this.hostConfig = {
        git_name: null,
        git_email: null,
        ssh_public_key: null,
        anthropic_api_key: null,
        google_api_key: null,
        openai_api_key: null,
        github_token: null,
        claude_oauth_credentials: null,
        google_adc: null,
      };
    } finally {
      this.hostConfigLoading = false;
    }
  }

  /** Auto-apply all detected values into settings, also enable their provider toggles. */
  private async applyDetected(config: HostConfig) {
    const mapping: { field: keyof HostConfig; settingId: string; toggleId?: string }[] = [
      { field: 'git_name', settingId: 'repository.git.identity.author_name' },
      { field: 'git_email', settingId: 'repository.git.identity.author_email' },
      { field: 'ssh_public_key', settingId: 'vm.environment.ssh.public_key' },
      { field: 'anthropic_api_key', settingId: 'ai.anthropic.api_key', toggleId: 'ai.anthropic.allow' },
      { field: 'google_api_key', settingId: 'ai.google.api_key', toggleId: 'ai.google.allow' },
      { field: 'openai_api_key', settingId: 'ai.openai.api_key', toggleId: 'ai.openai.allow' },
      { field: 'github_token', settingId: 'repository.providers.github.token' },
      { field: 'claude_oauth_credentials', settingId: 'ai.anthropic.claude.credentials_json', toggleId: 'ai.anthropic.allow' },
      { field: 'google_adc', settingId: 'ai.google.gemini.google_adc_json', toggleId: 'ai.google.allow' },
    ];

    const changes: Record<string, any> = {};
    const applied = new Set<string>();
    for (const { field, settingId, toggleId } of mapping) {
      const value = config[field];
      if (value) {
        changes[settingId] = value;
        applied.add(settingId);
        if (toggleId) {
          changes[toggleId] = true;
          applied.add(toggleId);
        }
      }
    }
    if (Object.keys(changes).length > 0) {
      try {
        await saveSettings(changes);
      } catch {
        // best-effort
      }
    }
    this.autoApplied = applied;
  }

  /** Clear an auto-applied setting back to empty. */
  async clearDetected(settingId: string) {
    await saveSettings({ [settingId]: '' });
    const next = new Set(this.autoApplied);
    next.delete(settingId);
    this.autoApplied = next;
  }

  /** Check if a setting was auto-applied from host detection. */
  wasAutoApplied(settingId: string): boolean {
    return this.autoApplied.has(settingId);
  }
}

export const wizardStore = new WizardStore();
