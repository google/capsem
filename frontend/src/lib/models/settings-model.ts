// Pure TypeScript settings model -- no Svelte dependency.
// Encapsulates parsing, accessors, validation, and pending state.

import {
  type SettingType as SettingTypeStr,
  type SettingValue,
  type SettingsNode,
  type SettingsGroup,
  type SettingsLeaf,
  type McpServerNode,
  type ConfigIssue,
  type SecurityPreset,
  type SettingsResponse,
} from '../types/settings';
import {
  SettingType,
  Widget,
  SideEffect,
  defaultWidget,
} from './settings-enums';

export class SettingsModel {
  private _tree: SettingsNode[];
  private _issues: ConfigIssue[];
  private _presets: SecurityPreset[];
  private _leafIndex: Map<string, SettingsLeaf>;
  private _mcpIndex: Map<string, McpServerNode>;
  private _pendingChanges: Map<string, SettingValue>;

  constructor(response: SettingsResponse) {
    this._tree = response.tree;
    this._issues = response.issues;
    this._presets = response.presets;
    this._leafIndex = new Map();
    this._mcpIndex = new Map();
    this._pendingChanges = new Map();
    this._buildIndexes(this._tree);
  }

  private _buildIndexes(nodes: SettingsNode[]): void {
    for (const node of nodes) {
      if (node.kind === 'leaf') {
        this._leafIndex.set(node.id, node);
      } else if (node.kind === 'group') {
        this._buildIndexes(node.children);
      } else if (node.kind === 'mcp_server') {
        this._mcpIndex.set(node.key, node);
      }
    }
  }

  // --- Tree accessors ---

  get tree(): SettingsNode[] {
    return this._tree;
  }

  get sections(): SettingsGroup[] {
    return this._tree.filter(
      (n): n is SettingsGroup => n.kind === 'group',
    );
  }

  get flatLeaves(): SettingsLeaf[] {
    return Array.from(this._leafIndex.values());
  }

  get mcpServers(): McpServerNode[] {
    return Array.from(this._mcpIndex.values());
  }

  getLeaf(id: string): SettingsLeaf | undefined {
    return this._leafIndex.get(id);
  }

  getGroup(name: string): SettingsGroup | undefined {
    const search = (nodes: SettingsNode[]): SettingsGroup | undefined => {
      for (const node of nodes) {
        if (node.kind === 'group') {
          if (node.name === name) return node;
          const found = search(node.children);
          if (found) return found;
        }
      }
      return undefined;
    };
    return search(this._tree);
  }

  getMcpServer(key: string): McpServerNode | undefined {
    return this._mcpIndex.get(key);
  }

  section(name: string): SettingsGroup | undefined {
    return this._tree.find(
      (n): n is SettingsGroup => n.kind === 'group' && n.name === name,
    );
  }

  // --- Issues ---

  get issues(): ConfigIssue[] {
    return this._issues;
  }

  issuesFor(id: string): ConfigIssue[] {
    return this._issues.filter((i) => i.id === id);
  }

  // --- Presets ---

  get presets(): SecurityPreset[] {
    return this._presets;
  }

  get activePresetId(): string | null {
    for (const preset of this._presets) {
      const allMatch = Object.entries(preset.settings).every(([id, val]) => {
        const leaf = this._leafIndex.get(id);
        if (!leaf) return false;
        return JSON.stringify(leaf.effective_value) === JSON.stringify(val);
      });
      if (allMatch) return preset.id;
    }
    return null;
  }

  // --- Computed state ---

  get needsSetup(): boolean {
    const apiKeyTypes: SettingTypeStr[] = ['apikey'];
    for (const leaf of this._leafIndex.values()) {
      if (
        apiKeyTypes.includes(leaf.setting_type) &&
        leaf.enabled &&
        typeof leaf.effective_value === 'string' &&
        leaf.effective_value.length > 0
      ) {
        return false;
      }
    }
    return true;
  }

  // --- Enabled / visibility ---

  isEnabled(id: string): boolean {
    const leaf = this._leafIndex.get(id);
    return leaf?.enabled ?? true;
  }

  isCorpLocked(id: string): boolean {
    const leaf = this._leafIndex.get(id);
    return leaf?.corp_locked ?? false;
  }

  getSideEffect(id: string): SideEffect | null {
    const leaf = this._leafIndex.get(id);
    if (!leaf?.metadata.side_effect) return null;
    const val = leaf.metadata.side_effect as string;
    if (Object.values(SideEffect).includes(val as SideEffect)) {
      return val as SideEffect;
    }
    return null;
  }

  getWidget(leaf: SettingsLeaf): Widget {
    if (leaf.metadata.widget) {
      const w = leaf.metadata.widget as string;
      if (Object.values(Widget).includes(w as Widget)) {
        return w as Widget;
      }
    }
    // Check deprecated format field
    if (
      leaf.setting_type === SettingType.Text &&
      leaf.metadata.format === 'domain_list'
    ) {
      return Widget.DomainChips;
    }
    // Text with choices -> select
    if (
      leaf.setting_type === SettingType.Text &&
      leaf.metadata.choices.length > 0
    ) {
      return Widget.Select;
    }
    return defaultWidget(leaf.setting_type as SettingType);
  }

  // --- Pending changes ---

  get pendingChanges(): Map<string, SettingValue> {
    return this._pendingChanges;
  }

  get isDirty(): boolean {
    return this._pendingChanges.size > 0;
  }

  stage(id: string, value: SettingValue): void {
    this._pendingChanges = new Map(this._pendingChanges);
    this._pendingChanges.set(id, value);
  }

  unstage(id: string): void {
    this._pendingChanges = new Map(this._pendingChanges);
    this._pendingChanges.delete(id);
  }

  clearPending(): void {
    this._pendingChanges = new Map();
  }

  getPendingAsRecord(): Record<string, SettingValue> {
    return Object.fromEntries(this._pendingChanges);
  }

  // --- Export / Import ---

  /** Serialize all leaf settings to a portable JSON string. */
  exportToJSON(): string {
    const settings: Record<string, { value: SettingValue; corp_locked: boolean }> = {};
    for (const [id, leaf] of this._leafIndex) {
      settings[id] = {
        value: leaf.effective_value,
        corp_locked: leaf.corp_locked,
      };
    }
    return JSON.stringify({ version: '1', exported_at: new Date().toISOString(), settings }, null, 2);
  }

  /** Parse an exported JSON string and return a map of changes to stage.
   *  Skips corp-locked settings and settings whose value already matches. */
  importFromJSON(json: string): Map<string, SettingValue> {
    let parsed: unknown;
    try {
      parsed = JSON.parse(json);
    } catch {
      throw new Error('Invalid JSON');
    }
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
      throw new Error('Invalid settings file: expected an object');
    }
    const obj = parsed as Record<string, unknown>;
    if (obj.version !== '1') {
      throw new Error(`Unsupported settings version: ${String(obj.version ?? 'missing')}`);
    }
    if (typeof obj.settings !== 'object' || obj.settings === null || Array.isArray(obj.settings)) {
      throw new Error('Invalid settings file: missing settings object');
    }
    const incoming = obj.settings as Record<string, unknown>;
    const changes = new Map<string, SettingValue>();
    for (const [id, entry] of Object.entries(incoming)) {
      const leaf = this._leafIndex.get(id);
      if (!leaf) continue; // unknown setting, skip
      if (leaf.corp_locked) continue; // corp-locked, skip
      // Extract value: accept both { value, corp_locked } and raw values
      let value: SettingValue;
      if (typeof entry === 'object' && entry !== null && !Array.isArray(entry) && 'value' in entry) {
        value = (entry as { value: SettingValue }).value;
      } else {
        value = entry as SettingValue;
      }
      // Skip if same as current
      if (JSON.stringify(leaf.effective_value) === JSON.stringify(value)) continue;
      changes.set(id, value);
    }
    return changes;
  }
}
