// Settings store -- loads settings tree and lint issues.
import { getSettingsTree, lintConfig, updateSetting } from '../api';
import type { ConfigIssue, SettingsNode, SettingsLeaf, SettingValue } from '../types';

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

  /** Get lint issues for a specific setting ID. */
  issuesFor(id: string): ConfigIssue[] {
    return this.issues.filter((i) => i.id === id);
  }

  async load() {
    this.loading = true;
    this.error = null;
    try {
      this.tree = await getSettingsTree();
      this.issues = await lintConfig();
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
