// Settings store -- loads resolved settings, groups by category.
import { getSettings, updateSetting } from '../api';
import type { ResolvedSetting, SettingValue } from '../types';

class SettingsStore {
  settings = $state<ResolvedSetting[]>([]);
  loading = $state(false);

  categories = $derived(
    [...new Set(this.settings.map((s) => s.category))].sort(),
  );

  byCategory(category: string): ResolvedSetting[] {
    return this.settings.filter((s) => s.category === category);
  }

  error = $state<string | null>(null);

  async load() {
    this.loading = true;
    this.error = null;
    try {
      this.settings = await getSettings();
    } catch (e) {
      console.error('Failed to load settings:', e);
      this.error = String(e);
    } finally {
      this.loading = false;
    }
  }

  async update(id: string, value: SettingValue) {
    await updateSetting(id, value);
    await this.load();
  }
}

export const settingsStore = new SettingsStore();
