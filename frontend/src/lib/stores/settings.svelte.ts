// Settings store -- thin Svelte wrapper around SettingsModel.
import {
  getSettingsTree,
  lintConfig,
  listPresets,
  applyPreset,
  loadSettings,
  saveSettings,
} from '../api';
import { SettingsModel } from '../models/settings-model';
import type {
  ConfigIssue,
  SecurityPreset,
  SettingsGroup,
  SettingsNode,
  SettingsLeaf,
  SettingValue,
} from '../types';

class SettingsStore {
  model = $state<SettingsModel | null>(null);
  applyingPreset = $state<string | null>(null);
  loading = $state(false);
  error = $state<string | null>(null);

  // --- Delegated accessors (backward-compatible) ---

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

  isDirty = $derived(this.model?.isDirty ?? false);

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

  // --- Load (unified) ---

  async load() {
    this.loading = true;
    this.error = null;
    try {
      const response = await loadSettings();
      this.model = new SettingsModel(response);
    } catch (e) {
      // Fallback to legacy 3-call approach if new command not available
      try {
        const [tree, issues, presets] = await Promise.all([
          getSettingsTree(),
          lintConfig(),
          listPresets(),
        ]);
        this.model = new SettingsModel({ tree, issues, presets });
      } catch (e2) {
        console.error('Failed to load settings:', e2);
        this.error = String(e2);
      }
    } finally {
      this.loading = false;
    }
  }

  // --- Mutations ---

  /** Stage a local change without persisting (for text/number/file fields). */
  stage(id: string, value: SettingValue) {
    this.model?.stage(id, value);
  }

  /** Persist all pending changes in one IPC call. */
  async save() {
    if (!this.model?.isDirty) return;
    const changes = this.model.getPendingAsRecord();
    this.loading = true;
    try {
      const response = await saveSettings(changes);
      this.model = new SettingsModel(response);
    } catch (e) {
      this.error = String(e);
    } finally {
      this.loading = false;
    }
  }

  /** Discard all pending changes and reload from backend. */
  async discard() {
    this.model?.clearPending();
    await this.load();
  }

  /** Stage + save immediately (for toggles that need instant feedback). */
  async updateImmediate(id: string, value: SettingValue) {
    this.model?.stage(id, value);
    await this.save();
  }

  async applySecurityPreset(id: string) {
    this.applyingPreset = id;
    try {
      await applyPreset(id);
      await this.load();
    } finally {
      this.applyingPreset = null;
    }
  }
}

export const settingsStore = new SettingsStore();
