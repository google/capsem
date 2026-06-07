// Pure TypeScript settings model -- no Svelte dependency.
// Encapsulates parsing, accessors, validation, and pending state.

import {
  type SettingValue,
  type SettingsNode,
  type SettingsGroup,
  type SettingsLeaf,
  type McpServerNode,
  type PolicyConfig,
  type PolicyCallback,
  type PolicyRuleConfig,
  type SettingsChangeValue,
  type ConfigIssue,
  type SecurityPreset,
  type SettingsResponse,
  type ProviderStatus,
  type ToolConfigSourceRecord,
} from '../types/settings';
import {
  SettingType,
  Widget,
  SideEffect,
  defaultWidget,
} from './settings-enums';

function normalizePolicyConfig(policy: PolicyConfig | undefined): PolicyConfig {
  return {
    mcp: policy?.mcp ?? {},
    http: policy?.http ?? {},
    dns: policy?.dns ?? {},
    model: policy?.model ?? {},
    hook: policy?.hook ?? {},
  };
}

export const POLICY_RULE_TYPES = ['mcp', 'http', 'dns', 'model', 'hook'] as const;
export type PolicyRuleType = (typeof POLICY_RULE_TYPES)[number];

export interface PolicyRuleEntry {
  key: string;
  type: PolicyRuleType;
  name: string;
  rule: PolicyRuleConfig;
  origin?: string;
}

const CALLBACKS_BY_TYPE: Record<PolicyRuleType, PolicyCallback[]> = {
  mcp: ['mcp.request', 'mcp.response'],
  http: ['http.request', 'http.response'],
  dns: ['dns.query', 'dns.response'],
  model: ['model.request', 'model.response', 'model.tool_call', 'model.tool_response'],
  hook: ['hook.decision'],
};

function policyRulesFor(config: PolicyConfig, type: PolicyRuleType): Record<string, PolicyRuleConfig> {
  return config[type] ?? {};
}

export function policyRuleKey(type: PolicyRuleType, name: string): string {
  return `policy.${type}.${name}`;
}

export function policyRuleNameFromParts(parts: string[]): string {
  const normalized = parts
    .join('_')
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '_')
    .replace(/_+/g, '_')
    .replace(/^_+|_+$/g, '');
  return normalized || 'rule';
}

function escapeCelString(value: string): string {
  return value.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
}

function parseDomainList(value: SettingValue): string[] {
  if (Array.isArray(value)) {
    return value
      .filter((item): item is string => typeof item === 'string')
      .map((item) => item.trim())
      .filter(Boolean);
  }
  if (typeof value !== 'string') return [];
  return value
    .split(',')
    .map((part) => part.trim())
    .filter(Boolean);
}

function domainCondition(domain: string): string {
  if (domain.startsWith('*.') && domain.length > 2) {
    return `request.host.endsWith(".${escapeCelString(domain.slice(2))}")`;
  }
  return `request.host == "${escapeCelString(domain)}"`;
}

function methodCondition(base: string, method: string): string {
  return `${base} && request.method == "${method}"`;
}

function isPolicyRuleConfig(value: unknown): value is PolicyRuleConfig {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
  const rule = value as Record<string, unknown>;
  return (
    typeof rule.on === 'string' &&
    typeof rule.if === 'string' &&
    typeof rule.decision === 'string' &&
    typeof rule.priority === 'number'
  );
}

export class SettingsModel {
  private _tree: SettingsNode[];
  private _issues: ConfigIssue[];
  private _presets: SecurityPreset[];
  private _policy: PolicyConfig;
  private _providers: ProviderStatus[];
  private _toolConfigSources: Record<string, ToolConfigSourceRecord>;
  private _leafIndex: Map<string, SettingsLeaf>;
  private _mcpIndex: Map<string, McpServerNode>;
  private _pendingChanges: Map<string, SettingsChangeValue>;

  constructor(response: SettingsResponse) {
    this._tree = response.tree;
    this._issues = response.issues;
    this._presets = response.presets;
    this._policy = normalizePolicyConfig(response.policy);
    this._providers = response.providers ?? [];
    this._toolConfigSources = response.tool_config_sources ?? {};
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

  get policy(): PolicyConfig {
    return this._policy;
  }

  get providers(): ProviderStatus[] {
    return this._providers;
  }

  get toolConfigSources(): Record<string, ToolConfigSourceRecord> {
    return this._toolConfigSources;
  }

  get policyRuleEntries(): PolicyRuleEntry[] {
    const entries: PolicyRuleEntry[] = [];
    for (const type of POLICY_RULE_TYPES) {
      for (const [name, rule] of Object.entries(policyRulesFor(this._policy, type))) {
        entries.push({
          key: policyRuleKey(type, name),
          type,
          name,
          rule,
        });
      }
    }
    return entries.sort((left, right) => {
      const priority = left.rule.priority - right.rule.priority;
      if (priority !== 0) return priority;
      return left.key.localeCompare(right.key);
    });
  }

  get generatedPolicyRuleEntries(): PolicyRuleEntry[] {
    const entries: PolicyRuleEntry[] = [];
    const seenKeys = new Set<string>();
    const addRule = (
      type: PolicyRuleType,
      name: string,
      rule: PolicyRuleConfig,
      origin: string,
    ) => {
      const key = policyRuleKey(type, name);
      if (seenKeys.has(key)) return;
      seenKeys.add(key);
      entries.push({
        key,
        type,
        name,
        rule,
        origin,
      });
    };

    const customBlock = this._leafIndex.get('security.web.custom_block');
    for (const domain of parseDomainList(customBlock?.effective_value ?? '')) {
      const name = policyRuleNameFromParts(['block', 'custom', domain]);
      addRule(
        'http',
        name,
        {
          on: 'http.request',
          if: domainCondition(domain),
          decision: 'block',
          priority: 100,
          reason: `Blocked by ${customBlock?.name ?? 'blocked domains'}`,
        },
        customBlock?.id ?? 'security.web.custom_block',
      );
    }

    const customAllow = this._leafIndex.get('security.web.custom_allow');
    for (const domain of parseDomainList(customAllow?.effective_value ?? '')) {
      const name = policyRuleNameFromParts(['allow', 'custom', domain]);
      addRule(
        'http',
        name,
        {
          on: 'http.request',
          if: domainCondition(domain),
          decision: 'allow',
          priority: 900,
          reason: `Allowed by ${customAllow?.name ?? 'allowed domains'}`,
        },
        customAllow?.id ?? 'security.web.custom_allow',
      );
    }

    for (const leaf of this._leafIndex.values()) {
      const rules = leaf.metadata.rules ?? {};
      if (leaf.setting_type !== 'bool' || Object.keys(rules).length === 0) {
        continue;
      }
      const baseDomains = Array.isArray(leaf.metadata.domains) ? leaf.metadata.domains : [];
      const enabled = leaf.effective_value === true;

      if (!enabled) {
        for (const domain of baseDomains) {
          const name = policyRuleNameFromParts(['block', leaf.id, domain]);
          addRule(
            'http',
            name,
            {
              on: 'http.request',
              if: domainCondition(domain),
              decision: 'block',
              priority: 200,
              reason: `${leaf.name} is disabled`,
            },
            leaf.id,
          );
        }
        continue;
      }

      for (const [ruleName, permissions] of Object.entries(rules)) {
        const ruleDomains = Array.isArray(permissions.domains) ? permissions.domains : [];
        const domains = ruleDomains.length > 0 ? ruleDomains : baseDomains;
        const allowedMethods: string[] = [];
        if (permissions.get) allowedMethods.push('GET');
        if (permissions.post) allowedMethods.push('POST');
        if (permissions.put) allowedMethods.push('PUT');
        if (permissions.delete) allowedMethods.push('DELETE');

        for (const domain of domains) {
          const hostCondition = domainCondition(domain);
          for (const method of allowedMethods) {
            const name = policyRuleNameFromParts(['allow', leaf.id, ruleName, domain, method]);
            addRule(
              'http',
              name,
              {
                on: 'http.request',
                if: methodCondition(hostCondition, method),
                decision: 'allow',
                priority: 800,
                reason: `${leaf.name} permits ${method} requests`,
              },
              leaf.id,
            );
          }
        }
      }
    }

    return entries.sort((left, right) => left.key.localeCompare(right.key));
  }

  callbacksForPolicyType(type: PolicyRuleType): PolicyCallback[] {
    return CALLBACKS_BY_TYPE[type];
  }

  stagePolicyRule(type: PolicyRuleType, name: string, rule: PolicyRuleConfig): void {
    this.stage(policyRuleKey(type, name), rule);
  }

  deletePolicyRule(type: PolicyRuleType, name: string): void {
    this.stage(policyRuleKey(type, name), null);
  }

  stageGeneratedPolicyRules(): number {
    for (const entry of this.generatedPolicyRuleEntries) {
      this.stage(entry.key, entry.rule);
    }
    return this.generatedPolicyRuleEntries.length;
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

  get pendingChanges(): Map<string, SettingsChangeValue> {
    return this._pendingChanges;
  }

  get isDirty(): boolean {
    return this._pendingChanges.size > 0;
  }

  stage(id: string, value: SettingsChangeValue): void {
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

  getPendingAsRecord(): Record<string, SettingsChangeValue> {
    return Object.fromEntries(this._pendingChanges);
  }

  // --- Export / Import ---

  /** Serialize all leaf settings and named policy rules to a portable JSON string. */
  exportToJSON(): string {
    const settings: Record<string, { value: SettingValue; corp_locked: boolean }> = {};
    for (const [id, leaf] of this._leafIndex) {
      settings[id] = {
        value: leaf.effective_value,
        corp_locked: leaf.corp_locked,
      };
    }
    return JSON.stringify(
      {
        version: '1',
        exported_at: new Date().toISOString(),
        settings,
        policy: this._policy,
      },
      null,
      2,
    );
  }

  /** Parse an exported JSON string and return a map of changes to stage.
   *  Skips corp-locked settings and settings whose value already matches. */
  importFromJSON(json: string): Map<string, SettingsChangeValue> {
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
    const changes = new Map<string, SettingsChangeValue>();
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

    if (obj.policy !== undefined) {
      if (typeof obj.policy !== 'object' || obj.policy === null || Array.isArray(obj.policy)) {
        throw new Error('Invalid settings file: policy must be an object');
      }
      const incomingPolicy = normalizePolicyConfig(obj.policy as PolicyConfig);
      for (const type of POLICY_RULE_TYPES) {
        for (const [name, rule] of Object.entries(policyRulesFor(incomingPolicy, type))) {
          if (!isPolicyRuleConfig(rule)) {
            throw new Error(`Invalid policy rule: ${policyRuleKey(type, name)}`);
          }
          const current = policyRulesFor(this._policy, type)[name];
          if (JSON.stringify(current) === JSON.stringify(rule)) continue;
          changes.set(policyRuleKey(type, name), rule);
        }
      }
    }

    return changes;
  }
}
