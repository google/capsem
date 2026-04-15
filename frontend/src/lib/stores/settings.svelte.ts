// Settings store -- thin Svelte wrapper around SettingsModel.
// Wired to gateway settings API.
import { SettingsModel } from '../models/settings-model';
import { getSettings, saveSettings, applyPreset, reloadConfig } from '../api';
import type {
  ConfigIssue,
  SecurityPreset,
  SettingsGroup,
  SettingsNode,
  SettingsLeaf,
  SettingValue,
} from '../types/settings';

class SettingsStore {
  model = $state<SettingsModel | null>(null);
  applyingPreset = $state<string | null>(null);
  loading = $state(false);
  error = $state<string | null>(null);

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

  // --- Load ---

  async load() {
    this.loading = true;
    this.error = null;
    try {
      const response = await getSettings();
      this.model = new SettingsModel(response);
    } catch (e) {
      console.error('Failed to load settings:', e);
      this.error = String(e);
    } finally {
      this.loading = false;
    }
  }

  // --- Mutations ---

  /** Stage a local change without persisting (for text/number/file fields). */
  stage(id: string, value: SettingValue) {
    this.model?.stage(id, value);
  }

  /** Persist all pending changes via the gateway settings API. */
  async save() {
    if (!this.model?.isDirty) return;
    const changes = this.model.getPendingAsRecord();
    this.loading = true;
    try {
      const response = await saveSettings(changes);
      this.model = new SettingsModel(response);
      await reloadConfig().catch(() => {});
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
    return changes.size;
  }

  async applySecurityPreset(id: string) {
    this.applyingPreset = id;
    try {
      const response = await applyPreset(id);
      this.model = new SettingsModel(response);
      await reloadConfig().catch(() => {});
    } catch (e) {
      this.error = String(e);
    } finally {
      this.applyingPreset = null;
    }
  }
}

export const settingsStore = new SettingsStore();
