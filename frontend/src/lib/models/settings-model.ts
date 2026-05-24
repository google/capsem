// Pure TypeScript settings model -- no Svelte dependency.
// Encapsulates parsing, accessors, validation, and pending state.

import {
  type SettingType as SettingTypeStr,
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

function normalizeSecurityPresets(response: SettingsResponse): SecurityPreset[] {
  if (Array.isArray(response.presets)) {
    return response.presets;
  }
  if (!Array.isArray(response.profile_presets)) {
    return [];
  }
  return response.profile_presets.map((preset) => ({
    id: preset.id,
    name: preset.name,
    description: preset.description,
    settings: preset.settings ?? {},
    mcp: null,
  }));
}

function normalizeSettingsTree(response: SettingsResponse): SettingsNode[] {
  return Array.isArray(response.tree) ? response.tree : [];
}

function normalizeSettingsIssues(response: SettingsResponse): ConfigIssue[] {
  return Array.isArray(response.issues) ? response.issues : [];
}

export const POLICY_RULE_TYPES = ['mcp', 'http', 'dns', 'model', 'hook'] as const;
export type PolicyRuleType = (typeof POLICY_RULE_TYPES)[number];
export const EDITABLE_POLICY_RULE_TYPES = ['mcp', 'http', 'dns', 'model'] as const;

export interface PolicyRuleEntry {
  key: string;
  type: PolicyRuleType;
  name: string;
  rule: PolicyRuleConfig;
  origin?: string;
  pending?: 'add' | 'update' | 'delete';
}

const CALLBACKS_BY_TYPE: Record<PolicyRuleType, PolicyCallback[]> = {
  mcp: ['mcp.request', 'mcp.response'],
  http: ['http.request', 'http.response'],
  dns: ['dns.query', 'dns.response'],
  model: ['model.request', 'model.response', 'model.tool_call', 'model.tool_response'],
  hook: ['hook.decision'],
};

const POLICY_DECISIONS = ['allow', 'ask', 'block', 'rewrite'] as const;
const POLICY_RULE_NAME_RE = /^[A-Za-z0-9_-]+$/;
const HEADER_NAME_RE = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;

function policyRulesFor(config: PolicyConfig, type: PolicyRuleType): Record<string, PolicyRuleConfig> {
  return config[type] ?? {};
}

function assertEditablePolicyRuleType(type: PolicyRuleType): void {
  if (!(EDITABLE_POLICY_RULE_TYPES as readonly string[]).includes(type)) {
    throw new Error(`${type} policy rules are not editable in this release`);
  }
}

export function policyRuleKey(type: PolicyRuleType, name: string): string {
  return `policy.${type}.${name}`;
}

export function parsePolicyRuleKey(key: string): { type: PolicyRuleType; name: string } | null {
  const parts = key.split('.');
  if (parts.length !== 3 || parts[0] !== 'policy') return null;
  const type = parts[1];
  const name = parts[2];
  if (!(POLICY_RULE_TYPES as readonly string[]).includes(type)) return null;
  if (!POLICY_RULE_NAME_RE.test(name)) return null;
  return { type: type as PolicyRuleType, name };
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

function optionalString(
  rule: Record<string, unknown>,
  field: 'reason' | 'rewrite_target' | 'rewrite_value',
): { present: boolean; value: string | null } {
  if (!Object.prototype.hasOwnProperty.call(rule, field) || rule[field] === null || rule[field] === undefined) {
    return { present: false, value: null };
  }
  if (typeof rule[field] !== 'string') {
    throw new Error(`${field} must be a string`);
  }
  return { present: true, value: rule[field].trim() };
}

function normalizeHeaderList(value: unknown, field: string): string[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) {
    throw new Error(`${field} must be an array of HTTP header names`);
  }
  const seen = new Set<string>();
  const headers: string[] = [];
  for (const item of value) {
    if (typeof item !== 'string') {
      throw new Error(`${field} must contain only HTTP header names`);
    }
    const header = item.trim().toLowerCase();
    if (!header) {
      throw new Error(`${field} contains an empty HTTP header name`);
    }
    if (!HEADER_NAME_RE.test(header)) {
      throw new Error(`${field} contains invalid HTTP header name '${item}'`);
    }
    if (!seen.has(header)) {
      seen.add(header);
      headers.push(header);
    }
  }
  return headers;
}

function rewriteTargetField(target: string): string {
  const [field, regexText] = target.split('=~');
  if (regexText === undefined) {
    throw new Error("rewrite_target must use '<field> =~ <regex>'");
  }
  const normalized = field.trim();
  const regex = regexText.trim();
  if (!normalized) {
    throw new Error('rewrite_target field must not be empty');
  }
  if (regex.length < 2 || !['"', "'"].includes(regex[0])) {
    throw new Error('rewrite_target regex must be quoted');
  }
  const quote = regex[0];
  const end = regex.lastIndexOf(quote);
  if (end === 0) {
    throw new Error('rewrite_target regex is missing a closing quote');
  }
  if (regex.slice(end + 1).trim()) {
    throw new Error('rewrite_target regex has trailing content after closing quote');
  }
  return normalized;
}

function validateReplacementReferences(target: string, value: string): void {
  const captures = new Set<string>();
  for (const match of target.matchAll(/\(\?P?<([A-Za-z_][A-Za-z0-9_]*)>/g)) {
    captures.add(match[1]);
  }
  for (const match of value.matchAll(/\$\{([A-Za-z_][A-Za-z0-9_]*)\}/g)) {
    if (!captures.has(match[1])) {
      throw new Error(`rewrite_value references unknown capture '${match[1]}'`);
    }
  }
}

function rewriteTargetAllowed(callback: PolicyCallback, field: string): boolean {
  if (callback === 'http.request') {
    return (
      field === 'request.url' ||
      field === 'request.path' ||
      field === 'request.query' ||
      field.startsWith('request.headers.')
    );
  }
  if (callback === 'http.response') {
    return field === 'response.status' || field.startsWith('response.headers.');
  }
  if (callback === 'dns.query' || callback === 'dns.response') {
    return field === 'answer.ip' || field === 'answer.ips';
  }
  if (callback === 'mcp.request') {
    return field === 'arguments' || field.startsWith('arguments.');
  }
  if (callback === 'mcp.response') {
    return (
      field === 'response.content' ||
      field === 'response.text' ||
      field.startsWith('response.')
    );
  }
  if (callback === 'model.response') {
    return ['response.text', 'text', 'content', 'thinking_content'].includes(field);
  }
  if (callback === 'model.tool_call') {
    return field === 'tool.arguments' || field === 'tool.name' || field === 'tool.call_id' || field.startsWith('tool.arguments.');
  }
  if (callback === 'model.tool_response') {
    return field === 'content' || field === 'response.content';
  }
  return false;
}

export function normalizePolicyRuleConfig(
  type: PolicyRuleType,
  name: string,
  value: unknown,
): PolicyRuleConfig {
  if (!POLICY_RULE_NAME_RE.test(name)) {
    throw new Error(`invalid policy rule name: ${name}`);
  }
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error(`Invalid policy rule: ${policyRuleKey(type, name)}`);
  }
  const rule = value as Record<string, unknown>;
  if (typeof rule.on !== 'string' || !CALLBACKS_BY_TYPE[type].includes(rule.on as PolicyCallback)) {
    throw new Error(`policy rule ${policyRuleKey(type, name)} uses callback for a different policy type`);
  }
  if (typeof rule.if !== 'string' || rule.if.trim() === '') {
    throw new Error(`policy rule ${policyRuleKey(type, name)} requires a non-empty CEL condition`);
  }
  if (typeof rule.decision !== 'string' || !(POLICY_DECISIONS as readonly string[]).includes(rule.decision)) {
    throw new Error(`policy rule ${policyRuleKey(type, name)} has an invalid decision`);
  }
  if (typeof rule.priority !== 'number' || !Number.isFinite(rule.priority)) {
    throw new Error(`policy rule ${policyRuleKey(type, name)} requires a numeric priority`);
  }

  const callback = rule.on as PolicyCallback;
  const decision = rule.decision as PolicyRuleConfig['decision'];
  const reason = optionalString(rule, 'reason');
  const rewriteTarget = optionalString(rule, 'rewrite_target');
  const rewriteValue = optionalString(rule, 'rewrite_value');
  const stripRequestHeaders = normalizeHeaderList(rule.strip_request_headers, 'strip_request_headers');
  const stripResponseHeaders = normalizeHeaderList(rule.strip_response_headers, 'strip_response_headers');

  const normalized: PolicyRuleConfig = {
    on: callback,
    if: rule.if.trim(),
    decision,
    priority: rule.priority,
  };
  if (reason.value) normalized.reason = reason.value;

  if (decision === 'rewrite') {
    const hasTarget = Boolean(rewriteTarget.value);
    const hasValue = Boolean(rewriteValue.value);
    const hasHeaderStrip = stripRequestHeaders.length > 0 || stripResponseHeaders.length > 0;
    if (stripRequestHeaders.length > 0 && callback !== 'http.request') {
      throw new Error(`strip_request_headers is only supported for http.request`);
    }
    if (stripResponseHeaders.length > 0 && callback !== 'http.response') {
      throw new Error(`strip_response_headers is only supported for http.response`);
    }
    if (hasTarget !== hasValue) {
      throw new Error('rewrite requires both rewrite_target and rewrite_value');
    }
    if (!hasTarget && !hasHeaderStrip) {
      throw new Error('rewrite requires rewrite_target/rewrite_value or header strip fields');
    }
    if (hasTarget && rewriteTarget.value && rewriteValue.value) {
      const field = rewriteTargetField(rewriteTarget.value);
      if (!rewriteTargetAllowed(callback, field)) {
        throw new Error(`unsupported rewrite target '${field}' for ${callback}`);
      }
      validateReplacementReferences(rewriteTarget.value, rewriteValue.value);
      normalized.rewrite_target = rewriteTarget.value;
      normalized.rewrite_value = rewriteValue.value;
    }
    if (stripRequestHeaders.length > 0) normalized.strip_request_headers = stripRequestHeaders;
    if (stripResponseHeaders.length > 0) normalized.strip_response_headers = stripResponseHeaders;
  } else if (
    rewriteTarget.present ||
    rewriteValue.present ||
    stripRequestHeaders.length > 0 ||
    stripResponseHeaders.length > 0
  ) {
    throw new Error('only rewrite decisions may carry rewrite fields');
  }

  return normalized;
}

export function validatePolicyRuleConfig(
  type: PolicyRuleType,
  name: string,
  value: unknown,
): string | null {
  try {
    normalizePolicyRuleConfig(type, name, value);
    return null;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
}

function assertNoDuplicatePolicyRuleKeys(json: string): void {
  let cursor = 0;

  function skipWhitespace() {
    while (/\s/.test(json[cursor] ?? '')) cursor += 1;
  }

  function parseString(): string {
    if (json[cursor] !== '"') throw new Error('invalid json string');
    cursor += 1;
    let value = '';
    while (cursor < json.length) {
      const ch = json[cursor++];
      if (ch === '"') return value;
      if (ch === '\\') {
        const escaped = json[cursor++];
        value += escaped ?? '';
      } else {
        value += ch;
      }
    }
    throw new Error('unterminated json string');
  }

  function parsePrimitive() {
    while (cursor < json.length && !/[\s,\]}]/.test(json[cursor])) cursor += 1;
  }

  function parseArray(path: string[]) {
    cursor += 1;
    skipWhitespace();
    if (json[cursor] === ']') {
      cursor += 1;
      return;
    }
    while (cursor < json.length) {
      parseValue(path);
      skipWhitespace();
      if (json[cursor] === ',') {
        cursor += 1;
        continue;
      }
      if (json[cursor] === ']') {
        cursor += 1;
        return;
      }
      throw new Error('invalid json array');
    }
  }

  function parseObject(path: string[]) {
    cursor += 1;
    const keys = new Set<string>();
    const detectDuplicates = path[0] === 'policy' && path.length === 2;
    skipWhitespace();
    if (json[cursor] === '}') {
      cursor += 1;
      return;
    }
    while (cursor < json.length) {
      skipWhitespace();
      const key = parseString();
      if (detectDuplicates) {
        if (keys.has(key)) {
          throw new Error(`Duplicate policy rule key: policy.${path[1]}.${key}`);
        }
        keys.add(key);
      }
      skipWhitespace();
      if (json[cursor] !== ':') throw new Error('invalid json object');
      cursor += 1;
      parseValue([...path, key]);
      skipWhitespace();
      if (json[cursor] === ',') {
        cursor += 1;
        continue;
      }
      if (json[cursor] === '}') {
        cursor += 1;
        return;
      }
      throw new Error('invalid json object');
    }
  }

  function parseValue(path: string[]) {
    skipWhitespace();
    const ch = json[cursor];
    if (ch === '{') return parseObject(path);
    if (ch === '[') return parseArray(path);
    if (ch === '"') {
      parseString();
      return;
    }
    parsePrimitive();
  }

  try {
    parseValue([]);
  } catch (error) {
    if (error instanceof Error && error.message.startsWith('Duplicate policy rule key:')) {
      throw error;
    }
  }
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
  private _selectedProfileId: string | null;
  private _leafIndex: Map<string, SettingsLeaf>;
  private _mcpIndex: Map<string, McpServerNode>;
  private _pendingChanges: Map<string, SettingsChangeValue>;

  constructor(response: SettingsResponse) {
    this._tree = normalizeSettingsTree(response);
    this._issues = normalizeSettingsIssues(response);
    this._presets = normalizeSecurityPresets(response);
    this._policy = normalizePolicyConfig(response.policy ?? response.effective_rules);
    this._selectedProfileId = response.settings_profiles?.selected_profile_id ?? null;
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

  get policyRuleEntries(): PolicyRuleEntry[] {
    const byKey = new Map<string, PolicyRuleEntry>();
    for (const type of POLICY_RULE_TYPES) {
      for (const [name, rule] of Object.entries(policyRulesFor(this._policy, type))) {
        const key = policyRuleKey(type, name);
        byKey.set(key, {
          key: policyRuleKey(type, name),
          type,
          name,
          rule,
        });
      }
    }
    for (const [key, value] of this._pendingChanges) {
      const parsed = parsePolicyRuleKey(key);
      if (!parsed) continue;
      const current = byKey.get(key);
      if (value === null) {
        if (current) {
          byKey.set(key, { ...current, pending: 'delete' });
        }
        continue;
      }
      if (!isPolicyRuleConfig(value)) continue;
      const rule = normalizePolicyRuleConfig(parsed.type, parsed.name, value);
      byKey.set(key, {
        key,
        type: parsed.type,
        name: parsed.name,
        rule,
        pending: current ? 'update' : 'add',
      });
    }
    const entries = Array.from(byKey.values());
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
      if (this.policyRuleMatchesPendingOrEffective(key, rule)) return;
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
    assertEditablePolicyRuleType(type);
    this.stage(policyRuleKey(type, name), normalizePolicyRuleConfig(type, name, rule));
  }

  stagePolicyRuleRename(oldKey: string, type: PolicyRuleType, name: string, rule: PolicyRuleConfig): void {
    assertEditablePolicyRuleType(type);
    const newKey = policyRuleKey(type, name);
    const normalized = normalizePolicyRuleConfig(type, name, rule);
    this._pendingChanges = new Map(this._pendingChanges);
    if (oldKey !== newKey) {
      this._pendingChanges.set(oldKey, null);
    }
    this._pendingChanges.set(newKey, normalized);
  }

  deletePolicyRule(type: PolicyRuleType, name: string): void {
    this.stage(policyRuleKey(type, name), null);
  }

  stageGeneratedPolicyRules(): number {
    const entries = this.generatedPolicyRuleEntries;
    for (const entry of entries) {
      this.stage(entry.key, entry.rule);
    }
    return entries.length;
  }

  private policyRuleMatchesPendingOrEffective(key: string, rule: PolicyRuleConfig): boolean {
    const parsed = parsePolicyRuleKey(key);
    if (!parsed) return false;
    const normalized = normalizePolicyRuleConfig(parsed.type, parsed.name, rule);
    if (this._pendingChanges.has(key)) {
      const pending = this._pendingChanges.get(key);
      return pending !== null && isPolicyRuleConfig(pending) && JSON.stringify(normalizePolicyRuleConfig(parsed.type, parsed.name, pending)) === JSON.stringify(normalized);
    }
    const current = policyRulesFor(this._policy, parsed.type)[parsed.name];
    return Boolean(current && JSON.stringify(normalizePolicyRuleConfig(parsed.type, parsed.name, current)) === JSON.stringify(normalized));
  }

  get activePresetId(): string | null {
    if (this._selectedProfileId) {
      for (const preset of this._presets) {
        if (preset.settings['profiles.default_profile'] === this._selectedProfileId) {
          return preset.id;
        }
      }
    }
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
      assertNoDuplicatePolicyRuleKeys(json);
    } catch (error) {
      if (error instanceof Error && error.message.startsWith('Duplicate policy rule key:')) {
        throw error;
      }
    }
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
          assertEditablePolicyRuleType(type);
          const normalizedRule = normalizePolicyRuleConfig(type, name, rule);
          const current = policyRulesFor(this._policy, type)[name];
          if (current && JSON.stringify(normalizePolicyRuleConfig(type, name, current)) === JSON.stringify(normalizedRule)) continue;
          changes.set(policyRuleKey(type, name), normalizedRule);
        }
      }
    }

    return changes;
  }
}
