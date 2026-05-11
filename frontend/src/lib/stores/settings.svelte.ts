// Settings store -- thin Svelte wrapper around SettingsModel.
// Wired to gateway settings API.
import { SettingsModel } from '../models/settings-model';
import { getSettings, saveSettings, applyPreset, reloadConfig, ReloadConfigError, type ReloadConfigResult } from '../api';
import type {
  ConfigIssue,
  SecurityPreset,
  SettingsGroup,
  SettingsNode,
  SettingsLeaf,
  SettingValue,
  PolicyRuleConfig,
  SettingsChangeValue,
} from '../types/settings';
import type { PolicyRuleType } from '../models/settings-model';

export type RuntimeReloadState = {
  persisted: boolean;
  applied: boolean;
  failed_session_count: number;
  failed_session_ids: string[];
  message: string | null;
  retry_available: boolean;
};

class SettingsStore {
  model = $state<SettingsModel | null>(null);
  applyingPreset = $state<string | null>(null);
  loading = $state(false);
  error = $state<string | null>(null);
  reloadError = $state<string | null>(null);
  reloadState = $state<RuntimeReloadState | null>(null);
  revision = $state(0);

  // --- Delegated accessors ---

  get tree(): SettingsNode[] {
    return this.model?.tree ?? [];
  }

  get issues(): ConfigIssue[] {
    return this.model?.issues ?? [];
  }

  get presets(): SecurityPreset[] {
    return this.model?.presets ?? [];
  }

  sections = $derived(
    this.model?.sections.map((g) => g.name) ?? [],
  );

  activePresetId = $derived(this.model?.activePresetId ?? null);

  needsSetup = $derived(this.model?.needsSetup ?? false);

  isDirty = $derived.by(() => {
    this.revision;
    return this.model?.isDirty ?? false;
  });

  section(name: string): SettingsGroup | undefined {
    return this.model?.section(name);
  }

  findLeaf(id: string): SettingsLeaf | undefined {
    return this.model?.getLeaf(id);
  }

  findGroup(name: string): SettingsGroup | undefined {
    return this.model?.getGroup(name);
  }

  issuesFor(id: string): ConfigIssue[] {
    return this.model?.issuesFor(id) ?? [];
  }

  // --- Load ---

  async load() {
    this.loading = true;
    this.error = null;
    this.reloadError = null;
    this.reloadState = null;
    try {
      const response = await getSettings();
      this.model = new SettingsModel(response);
      this.touch();
    } catch (e) {
      console.error('Failed to load settings:', e);
      this.error = String(e);
    } finally {
      this.loading = false;
    }
  }

  // --- Mutations ---

  /** Stage a local change without persisting (for text/number/file fields). */
  stage(id: string, value: SettingsChangeValue) {
    this.clearRuntimeReloadState();
    this.model?.stage(id, value);
    this.touch();
  }

  stagePolicyRule(type: PolicyRuleType, name: string, rule: PolicyRuleConfig) {
    this.clearRuntimeReloadState();
    this.model?.stagePolicyRule(type, name, rule);
    this.touch();
  }

  stagePolicyRuleRename(oldKey: string, type: PolicyRuleType, name: string, rule: PolicyRuleConfig) {
    this.clearRuntimeReloadState();
    this.model?.stagePolicyRuleRename(oldKey, type, name, rule);
    this.touch();
  }

  deletePolicyRule(type: PolicyRuleType, name: string) {
    this.clearRuntimeReloadState();
    this.model?.deletePolicyRule(type, name);
    this.touch();
  }

  stageGeneratedPolicyRules(): number {
    this.clearRuntimeReloadState();
    const count = this.model?.stageGeneratedPolicyRules() ?? 0;
    if (count > 0) this.touch();
    return count;
  }

  /** Persist all pending changes via the gateway settings API. */
  async save() {
    if (!this.model?.isDirty) return;
    const changes = this.model.getPendingAsRecord();
    this.loading = true;
    this.error = null;
    this.reloadError = null;
    this.reloadState = null;
    try {
      const response = await saveSettings(changes);
      this.model = new SettingsModel(response);
      this.touch();
      await this.reloadRuntime();
    } catch (e) {
      this.error = String(e);
    } finally {
      this.loading = false;
    }
  }

  /** Discard all pending changes and reload. */
  async discard() {
    this.model?.clearPending();
    await this.load();
  }

  /** Stage + save immediately (for toggles that need instant feedback). */
  async updateImmediate(id: string, value: SettingValue) {
    this.model?.stage(id, value);
    await this.save();
  }

  /** Export all settings as a JSON file download. */
  exportSettings() {
    if (!this.model) return;
    const json = this.model.exportToJSON();
    const blob = new Blob([json], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'capsem-settings.json';
    a.click();
    URL.revokeObjectURL(url);
  }

  /** Import settings from a JSON file. Changes are staged (not saved). */
  async importSettings(file: File): Promise<number> {
    if (!this.model) throw new Error('Settings not loaded');
    const text = await file.text();
    const changes = this.model.importFromJSON(text);
    for (const [id, value] of changes) {
      this.model.stage(id, value);
    }
    if (changes.size > 0) this.clearRuntimeReloadState();
    if (changes.size > 0) this.touch();
    return changes.size;
  }

  async applySecurityPreset(id: string) {
    this.applyingPreset = id;
    this.error = null;
    this.reloadError = null;
    this.reloadState = null;
    try {
      const response = await applyPreset(id);
      this.model = new SettingsModel(response);
      this.touch();
      await this.reloadRuntime();
    } catch (e) {
      this.error = String(e);
    } finally {
      this.applyingPreset = null;
    }
  }

  async retryReload() {
    this.loading = true;
    try {
      this.reloadError = null;
      await this.reloadRuntime();
    } finally {
      this.loading = false;
    }
  }

  clearReloadStateIfAffectedSessionsStopped(activeSessionIds: Iterable<string>) {
    const state = this.reloadState;
    if (!state || state.applied || state.failed_session_ids.length === 0) {
      return;
    }
    const active = new Set(activeSessionIds);
    if (state.failed_session_ids.every((id) => !active.has(id))) {
      this.clearRuntimeReloadState();
    }
  }

  private async reloadRuntime() {
    try {
      const result = await reloadConfig();
      this.reloadError = null;
      this.reloadState = this.reloadStateFromResult(result, true);
    } catch (e) {
      const result = this.reloadResultFromError(e);
      const message = result.message ?? String(e);
      this.reloadState = this.reloadStateFromResult(result, false);
      this.reloadError = `Saved, but the running service did not reload: ${message}`;
    }
  }

  private touch() {
    this.revision += 1;
  }

  private clearRuntimeReloadState() {
    this.reloadError = null;
    this.reloadState = null;
  }

  private reloadStateFromResult(result: ReloadConfigResult, applied: boolean): RuntimeReloadState {
    return {
      persisted: true,
      applied,
      failed_session_count: result.failed_session_count,
      failed_session_ids: result.failed_session_ids,
      message: result.message,
      retry_available: !applied,
    };
  }

  private reloadResultFromError(error: unknown): ReloadConfigResult {
    if (error instanceof ReloadConfigError) {
      return error.result;
    }
    const maybe = error as { result?: ReloadConfigResult };
    if (maybe?.result) {
      return maybe.result;
    }
    return {
      success: false,
      reloaded: 0,
      failed_session_count: 0,
      failed_session_ids: [],
      failures: [],
      message: error instanceof Error ? error.message : String(error),
    };
  }
}

export const settingsStore = new SettingsStore();
