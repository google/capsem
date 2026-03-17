// Settings store -- loads settings tree and lint issues.
import { getSettingsTree, lintConfig, listPresets, applyPreset, updateSetting } from '../api';
import type { ConfigIssue, SecurityPreset, SettingsNode, SettingsLeaf, SettingValue } from '../types';

/** Recursively collect all leaf nodes from a settings tree. */
function flattenLeaves(nodes: SettingsNode[]): SettingsLeaf[] {
  const leaves: SettingsLeaf[] = [];
  for (const node of nodes) {
    if (node.kind === 'leaf') {
      leaves.push(node);
    } else {
      leaves.push(...flattenLeaves(node.children));
    }
  }
  return leaves;
}

class SettingsStore {
  tree = $state<SettingsNode[]>([]);
  issues = $state<ConfigIssue[]>([]);
  presets = $state<SecurityPreset[]>([]);
  applyingPreset = $state<string | null>(null);
  loading = $state(false);
  error = $state<string | null>(null);

  /** Top-level section names derived from the tree. */
  sections = $derived(
    this.tree
      .filter((n): n is Extract<SettingsNode, { kind: 'group' }> => n.kind === 'group')
      .map((g) => g.name),
  );

  /** All leaf settings (for needsSetup and other flat lookups). */
  private flatLeaves = $derived(flattenLeaves(this.tree));

  /** ID of the preset that matches current effective values, or null if modified. */
  activePresetId = $derived.by(() => {
    if (this.presets.length === 0 || this.tree.length === 0) return null;
    const leafMap = new Map(this.flatLeaves.map(l => [l.id, l.effective_value]));
    for (const preset of this.presets) {
      const allMatch = Object.entries(preset.settings).every(
        ([id, val]) => leafMap.has(id) && leafMap.get(id) === val,
      );
      if (allMatch) return preset.id;
    }
    return null;
  });

  /** True when no enabled API key is configured. */
  needsSetup = $derived(
    this.flatLeaves.length > 0 &&
    !this.flatLeaves.some(
      (s) => s.setting_type === 'apikey' && s.enabled && String(s.effective_value).trim().length > 0
    )
  );

  /** Find a top-level group by name. */
  section(name: string): SettingsNode | undefined {
    return this.tree.find(
      (n) => n.kind === 'group' && n.name === name,
    );
  }

  /** Find a leaf setting by its ID. */
  findLeaf(id: string): SettingsLeaf | undefined {
    return this.flatLeaves.find((l) => l.id === id);
  }

  /** Find a group anywhere in the tree by name (searches recursively). */
  findGroup(name: string): SettingsNode | undefined {
    function search(nodes: SettingsNode[]): SettingsNode | undefined {
      for (const node of nodes) {
        if (node.kind === 'group' && node.name === name) return node;
        if (node.kind === 'group') {
          const found = search(node.children);
          if (found) return found;
        }
      }
      return undefined;
    }
    return search(this.tree);
  }

  /** Get lint issues for a specific setting ID. */
  issuesFor(id: string): ConfigIssue[] {
    return this.issues.filter((i) => i.id === id);
  }

  async load() {
    this.loading = true;
    this.error = null;
    try {
      const [tree, issues, presets] = await Promise.all([
        getSettingsTree(),
        lintConfig(),
        listPresets(),
      ]);
      this.tree = tree;
      this.issues = issues;
      this.presets = presets;
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
