import { describe, it, expect } from 'vitest';
import {
  EDITABLE_POLICY_RULE_TYPES,
  SettingsModel,
  policyRuleKey,
  validatePolicyRuleConfig,
} from '../settings-model';
import { Widget } from '../settings-enums';
import { buildMockSettingsResponse } from '../../mock-settings';

function loadModel(): SettingsModel {
  return new SettingsModel(buildMockSettingsResponse());
}

describe('SettingsModel', () => {
  describe('tree indexing', () => {
    it('loads the Profile V2 /settings envelope without legacy tree fields', () => {
      const model = new SettingsModel({
        mode: 'settings_profiles_v2',
        profile_presets: [
          {
            id: 'coding',
            name: 'Coding',
            description: 'Focused defaults for software development sessions.',
            settings: { 'profiles.default_profile': 'coding' },
          },
          {
            id: 'everyday-work',
            name: 'Everyday Work',
            description: 'Balanced defaults for daily work sessions.',
            settings: { 'profiles.default_profile': 'everyday-work' },
          },
        ],
        settings_profiles: {
          selected_profile_id: 'everyday-work',
          effective: {
            profile_id: 'everyday-work',
          },
        },
        effective_rules: {
          http: {
            block_example: {
              on: 'http.request',
              if: 'http.request.host == "example.com"',
              decision: 'block',
              priority: 10,
              reason: 'test rule',
            },
          },
        },
      });

      expect(model.tree).toEqual([]);
      expect(model.issues).toEqual([]);
      expect(model.presets.map((preset) => preset.id)).toEqual(['coding', 'everyday-work']);
      expect(model.activePresetId).toBe('everyday-work');
      expect(model.policyRuleEntries).toHaveLength(1);
      expect(model.policyRuleEntries[0]).toMatchObject({
        key: 'policy.http.block_example',
        type: 'http',
        name: 'block_example',
      });
    });

    it('finds leaf settings by ID', () => {
      const model = loadModel();
      const leaf = model.getLeaf('ai.anthropic.allow');
      expect(leaf).toBeDefined();
      expect(leaf!.name).toBe('Allow Anthropic');
    });

    it('returns undefined for unknown ID', () => {
      const model = loadModel();
      expect(model.getLeaf('nonexistent')).toBeUndefined();
    });

    it('indexes all leaf settings', () => {
      const model = loadModel();
      expect(model.flatLeaves.length).toBeGreaterThan(30);
    });
  });

  describe('sections', () => {
    it('returns top-level groups', () => {
      const model = loadModel();
      const names = model.sections.map(s => s.name);
      expect(names).toContain('AI Providers');
      expect(names).toContain('Repositories');
      expect(names).toContain('Security');
      expect(names).toContain('VM');
    });

    it('section() finds by name', () => {
      const model = loadModel();
      const ai = model.section('AI Providers');
      expect(ai).toBeDefined();
      expect(ai!.key).toBe('ai');
    });
  });

  describe('getGroup', () => {
    it('finds nested groups', () => {
      const model = loadModel();
      const claude = model.getGroup('Claude Code');
      expect(claude).toBeDefined();
      expect(claude!.key).toBe('ai.anthropic.claude');
    });
  });

  describe('issues', () => {
    it('filters issues by ID', () => {
      const model = loadModel();
      const issues = model.issuesFor('ai.anthropic.api_key');
      expect(issues.length).toBeGreaterThan(0);
      expect(issues[0].severity).toBe('warning');
    });

    it('returns empty for IDs without issues', () => {
      const model = loadModel();
      expect(model.issuesFor('vm.resources.cpu_count')).toEqual([]);
    });
  });

  describe('presets', () => {
    it('has presets available', () => {
      const model = loadModel();
      expect(model.presets.length).toBeGreaterThan(0);
    });

    it('activePresetId detects matching preset', () => {
      const model = loadModel();
      // Default mock settings match the "high" preset
      expect(model.activePresetId).toBe('high');
    });
  });

  describe('policy', () => {
    it('normalizes omitted policy maps from the settings response', () => {
      const response = buildMockSettingsResponse();
      response.policy = {
        http: {
          block_openai_github: {
            on: 'http.request',
            if: "request.host == 'github.com'",
            decision: 'block',
            priority: 10,
          },
        },
      };

      const model = new SettingsModel(response);
      expect(model.policy.mcp).toEqual({});
      expect(Object.keys(model.policy.http ?? {})).toEqual([
        'block_openai_github',
      ]);
      expect(model.policy.dns).toEqual({});
      expect(model.policy.model).toEqual({});
      expect(model.policy.hook).toEqual({});
    });

    it('lists named policy rules with full settings-save keys', () => {
      const model = loadModel();
      const keys = model.policyRuleEntries.map((entry) => entry.key);
      expect(keys).toContain('policy.http.block_openai_github');
      expect(keys).toContain('policy.mcp.ask_prod_issue');
    });

    it('keeps hook readable but out of editable release rule types', () => {
      expect(EDITABLE_POLICY_RULE_TYPES).toEqual(['mcp', 'http', 'dns', 'model']);
      const model = loadModel();
      expect(model.callbacksForPolicyType('hook')).toEqual(['hook.decision']);
      expect(() => model.stagePolicyRule('hook', 'external_decision', {
        on: 'hook.decision',
        if: 'decision == "block"',
        decision: 'block',
        priority: 10,
      })).toThrow('hook policy rules are not editable in this release');
    });

    it('merges staged policy additions, updates, and deletes into review entries', () => {
      const model = loadModel();
      model.stagePolicyRule('http', 'block_evil', {
        on: 'http.request',
        if: 'request.host == "evil.com"',
        decision: 'block',
        priority: 5,
      });
      model.stagePolicyRule('mcp', 'ask_prod_issue', {
        on: 'mcp.request',
        if: 'method == "tools/call" && arguments.issue == "prod"',
        decision: 'block',
        priority: 4,
      });
      model.deletePolicyRule('http', 'block_openai_github');

      const entries = model.policyRuleEntries;
      expect(entries.find((entry) => entry.key === 'policy.http.block_evil')?.pending).toBe('add');
      expect(entries.find((entry) => entry.key === 'policy.mcp.ask_prod_issue')?.pending).toBe('update');
      expect(entries.find((entry) => entry.key === 'policy.http.block_openai_github')?.pending).toBe('delete');
    });

    it('stages rename and type change atomically', () => {
      const model = loadModel();
      model.stagePolicyRuleRename('policy.http.block_openai_github', 'mcp', 'block_prod_tool', {
        on: 'mcp.request',
        if: 'method == "tools/call"',
        decision: 'block',
        priority: 5,
      });

      expect(model.pendingChanges.get('policy.http.block_openai_github')).toBeNull();
      expect(model.pendingChanges.get('policy.mcp.block_prod_tool')).toMatchObject({
        on: 'mcp.request',
        decision: 'block',
      });
      expect(model.policyRuleEntries.find((entry) => entry.key === 'policy.mcp.block_prod_tool')?.pending).toBe('add');
      expect(model.policyRuleEntries.find((entry) => entry.key === 'policy.http.block_openai_github')?.pending).toBe('delete');
    });

    it('generates Policy block rules from blocked domain chips', () => {
      const model = loadModel();
      const blocked = model.getLeaf('security.web.custom_block')!;
      (blocked as { effective_value: string }).effective_value = 'evil.com, *.tracker.example';

      const generated = model.generatedPolicyRuleEntries;
      const exact = generated.find((entry) => entry.key === 'policy.http.block_custom_evil_com');
      expect(exact?.rule).toEqual({
        on: 'http.request',
        if: 'request.host == "evil.com"',
        decision: 'block',
        priority: 100,
        reason: 'Blocked by Blocked domains',
      });

      const wildcard = generated.find((entry) => entry.key === 'policy.http.block_custom_tracker_example');
      expect(wildcard?.rule.if).toBe('request.host.endsWith(".tracker.example")');
    });

    it('generates method-aware Policy allow rules from metadata rules', () => {
      const model = loadModel();
      const generated = model.generatedPolicyRuleEntries;
      const key = policyRuleKey(
        'http',
        'allow_repository_providers_github_allow_default_github_com_post',
      );
      const rule = generated.find((entry) => entry.key === key)?.rule;
      expect(rule).toMatchObject({
        on: 'http.request',
        if: 'request.host == "github.com" && request.method == "POST"',
        decision: 'allow',
        priority: 800,
      });
    });

    it('deduplicates generated policy rules with the same key', () => {
      const model = loadModel();
      const allowed = model.getLeaf('security.web.custom_allow')!;
      (allowed as { effective_value: string }).effective_value = 'elie.net, elie.net';

      const generated = model.generatedPolicyRuleEntries.filter(
        (entry) => entry.key === 'policy.http.allow_custom_elie_net',
      );
      expect(generated).toHaveLength(1);
    });

    it('suppresses generated policy rules already effective or staged unchanged', () => {
      const response = buildMockSettingsResponse();
      response.policy!.http!.block_custom_evil_com = {
        on: 'http.request',
        if: 'request.host == "evil.com"',
        decision: 'block',
        priority: 100,
        reason: 'Blocked by Blocked domains',
      };
      const model = new SettingsModel(response);
      const blocked = model.getLeaf('security.web.custom_block')!;
      (blocked as { effective_value: string }).effective_value = 'evil.com, tracker.example';

      expect(model.generatedPolicyRuleEntries.map((entry) => entry.key)).not.toContain('policy.http.block_custom_evil_com');
      expect(model.generatedPolicyRuleEntries.map((entry) => entry.key)).toContain('policy.http.block_custom_tracker_example');

      const count = model.stageGeneratedPolicyRules();
      expect(count).toBeGreaterThan(0);
      expect(model.generatedPolicyRuleEntries.map((entry) => entry.key)).not.toContain('policy.http.block_custom_tracker_example');
    });

    it('validates policy rules before staging', () => {
      expect(validatePolicyRuleConfig('model', 'bad_callback', {
        on: 'http.request',
        if: 'request.host == "example.com"',
        decision: 'block',
        priority: 1,
      })).toContain('different policy type');
      expect(validatePolicyRuleConfig('http', 'bad_decision', {
        on: 'http.request',
        if: 'request.host == "example.com"',
        decision: 'deny',
        priority: 1,
      })).toContain('invalid decision');
      expect(validatePolicyRuleConfig('http', 'bad_header', {
        on: 'http.request',
        if: 'request.host == "example.com"',
        decision: 'rewrite',
        priority: 1,
        strip_request_headers: [''],
      })).toContain('empty HTTP header');
      expect(validatePolicyRuleConfig('http', 'bad_rewrite', {
        on: 'http.request',
        if: 'request.host == "example.com"',
        decision: 'rewrite',
        priority: 1,
        rewrite_target: 'response.body =~ "secret"',
        rewrite_value: '[redacted]',
      })).toContain('unsupported rewrite target');
    });

    it('tolerates omitted metadata arrays from live settings responses', () => {
      const model = loadModel();
      const leaf = model.getLeaf('repository.providers.github.allow')!;
      (leaf.metadata as { domains?: string[] }).domains = undefined;
      for (const permissions of Object.values(leaf.metadata.rules)) {
        (permissions as { domains?: string[] }).domains = undefined;
      }

      expect(() => model.generatedPolicyRuleEntries).not.toThrow();
    });
  });

  describe('getWidget', () => {
    it('returns Toggle for bool type', () => {
      const model = loadModel();
      const leaf = model.getLeaf('ai.anthropic.allow')!;
      expect(model.getWidget(leaf)).toBe(Widget.Toggle);
    });

    it('returns PasswordInput for apikey type', () => {
      const model = loadModel();
      const leaf = model.getLeaf('ai.anthropic.api_key')!;
      expect(model.getWidget(leaf)).toBe(Widget.PasswordInput);
    });

    it('returns FileEditor for file type', () => {
      const model = loadModel();
      const leaf = model.getLeaf('ai.anthropic.claude.settings_json')!;
      expect(model.getWidget(leaf)).toBe(Widget.FileEditor);
    });

    it('returns NumberInput for number type', () => {
      const model = loadModel();
      const leaf = model.getLeaf('vm.resources.cpu_count')!;
      expect(model.getWidget(leaf)).toBe(Widget.NumberInput);
    });

    it('returns DomainChips for format=domain_list', () => {
      const model = loadModel();
      const leaf = model.getLeaf('repository.providers.github.domains')!;
      expect(model.getWidget(leaf)).toBe(Widget.DomainChips);
    });

    it('returns TextInput for plain text', () => {
      const model = loadModel();
      const leaf = model.getLeaf('vm.environment.shell.term')!;
      expect(model.getWidget(leaf)).toBe(Widget.TextInput);
    });
  });

  describe('pending changes', () => {
    it('starts clean', () => {
      const model = loadModel();
      expect(model.isDirty).toBe(false);
      expect(model.pendingChanges.size).toBe(0);
    });

    it('stage marks dirty', () => {
      const model = loadModel();
      model.stage('vm.resources.cpu_count', 8);
      expect(model.isDirty).toBe(true);
      expect(model.pendingChanges.size).toBe(1);
    });

    it('clearPending resets', () => {
      const model = loadModel();
      model.stage('vm.resources.cpu_count', 8);
      model.clearPending();
      expect(model.isDirty).toBe(false);
    });

    it('unstage removes single change', () => {
      const model = loadModel();
      model.stage('vm.resources.cpu_count', 8);
      model.stage('vm.resources.ram_gb', 16);
      model.unstage('vm.resources.cpu_count');
      expect(model.pendingChanges.size).toBe(1);
      expect(model.pendingChanges.has('vm.resources.ram_gb')).toBe(true);
    });

    it('getPendingAsRecord returns plain object', () => {
      const model = loadModel();
      model.stage('vm.resources.cpu_count', 8);
      const record = model.getPendingAsRecord();
      expect(record).toEqual({ 'vm.resources.cpu_count': 8 });
    });

    it('stages policy rule objects for settings save', () => {
      const model = loadModel();
      const rule = {
        on: 'http.request' as const,
        if: "request.host == 'github.com'",
        decision: 'block' as const,
        priority: 10,
      };
      model.stage('policy.http.block_openai_github', rule);
      expect(model.getPendingAsRecord()).toEqual({
        'policy.http.block_openai_github': rule,
      });
    });
  });

  describe('needsSetup', () => {
    it('returns true when no API keys are set', () => {
      const model = loadModel();
      expect(model.needsSetup).toBe(true);
    });
  });

  describe('enabled / visibility', () => {
    it('isEnabled returns true for settings without enabled_by', () => {
      const model = loadModel();
      expect(model.isEnabled('ai.anthropic.allow')).toBe(true);
    });

    it('isCorpLocked returns false for normal settings', () => {
      const model = loadModel();
      expect(model.isCorpLocked('vm.resources.cpu_count')).toBe(false);
    });

    it('isCorpLocked returns true for locked settings', () => {
      const model = loadModel();
      const leaf = model.getLeaf('vm.resources.cpu_count');
      if (leaf) (leaf as { corp_locked: boolean }).corp_locked = true;
      expect(model.isCorpLocked('vm.resources.cpu_count')).toBe(true);
    });

    it('isEnabled returns true for unknown ID', () => {
      const model = loadModel();
      expect(model.isEnabled('nonexistent')).toBe(true);
    });

    it('isCorpLocked returns false for unknown ID', () => {
      const model = loadModel();
      expect(model.isCorpLocked('nonexistent')).toBe(false);
    });
  });

  describe('MCP servers', () => {
    it('mcpServers returns array', () => {
      const model = loadModel();
      expect(Array.isArray(model.mcpServers)).toBe(true);
    });

    it('getMcpServer returns undefined for unknown key', () => {
      const model = loadModel();
      expect(model.getMcpServer('nonexistent')).toBeUndefined();
    });
  });

  describe('side effects', () => {
    it('getSideEffect returns ToggleTheme for dark_mode', () => {
      const model = loadModel();
      expect(model.getSideEffect('appearance.dark_mode')).toBe('toggle_theme');
    });

    it('getSideEffect returns null for normal settings', () => {
      const model = loadModel();
      expect(model.getSideEffect('vm.resources.cpu_count')).toBeNull();
    });

    it('getSideEffect returns null for unknown ID', () => {
      const model = loadModel();
      expect(model.getSideEffect('nonexistent')).toBeNull();
    });
  });

  describe('pending changes edge cases', () => {
    it('stage then unstage leaves clean', () => {
      const model = loadModel();
      model.stage('vm.resources.cpu_count', 8);
      model.unstage('vm.resources.cpu_count');
      expect(model.isDirty).toBe(false);
      expect(model.pendingChanges.size).toBe(0);
    });

    it('unstage non-existent key is no-op', () => {
      const model = loadModel();
      model.unstage('nonexistent');
      expect(model.isDirty).toBe(false);
    });

    it('stage overwrites previous staged value', () => {
      const model = loadModel();
      model.stage('vm.resources.cpu_count', 2);
      model.stage('vm.resources.cpu_count', 8);
      expect(model.pendingChanges.get('vm.resources.cpu_count')).toBe(8);
      expect(model.pendingChanges.size).toBe(1);
    });

    it('clearPending after multiple stages', () => {
      const model = loadModel();
      model.stage('vm.resources.cpu_count', 8);
      model.stage('vm.resources.ram_gb', 16);
      model.stage('security.web.allow_read', true);
      model.clearPending();
      expect(model.isDirty).toBe(false);
      expect(model.pendingChanges.size).toBe(0);
    });

    it('getPendingAsRecord includes all staged changes', () => {
      const model = loadModel();
      model.stage('vm.resources.cpu_count', 8);
      model.stage('vm.resources.ram_gb', 16);
      const record = model.getPendingAsRecord();
      expect(record).toEqual({
        'vm.resources.cpu_count': 8,
        'vm.resources.ram_gb': 16,
      });
    });

    it('stage complex file value', () => {
      const model = loadModel();
      const fileVal = { path: '/root/.bashrc', content: '# test' };
      model.stage('vm.environment.shell.bashrc', fileVal);
      expect(model.pendingChanges.get('vm.environment.shell.bashrc')).toEqual(fileVal);
    });

    it('stage boolean false', () => {
      const model = loadModel();
      model.stage('ai.anthropic.allow', false);
      expect(model.pendingChanges.get('ai.anthropic.allow')).toBe(false);
    });

    it('stage number zero', () => {
      const model = loadModel();
      model.stage('vm.resources.cpu_count', 0);
      expect(model.pendingChanges.get('vm.resources.cpu_count')).toBe(0);
    });

    it('stage empty string', () => {
      const model = loadModel();
      model.stage('vm.environment.shell.term', '');
      expect(model.pendingChanges.get('vm.environment.shell.term')).toBe('');
    });
  });

  describe('tree structure', () => {
    it('flatLeaves count is consistent with tree', () => {
      const model = loadModel();
      expect(model.flatLeaves.length).toBeGreaterThan(30);
      // Every flat leaf should be findable by ID
      for (const leaf of model.flatLeaves) {
        expect(model.getLeaf(leaf.id)).toBe(leaf);
      }
    });

    it('sections are top-level groups only', () => {
      const model = loadModel();
      for (const section of model.sections) {
        expect(section.kind).toBe('group');
      }
    });

    it('tree contains various node kinds', () => {
      const model = loadModel();
      const kinds = new Set<string>();
      function walk(nodes: import('../../types/settings').SettingsNode[]) {
        for (const n of nodes) {
          kinds.add(n.kind);
          if (n.kind === 'group') walk(n.children);
        }
      }
      walk(model.tree);
      expect(kinds.has('group')).toBe(true);
      expect(kinds.has('leaf')).toBe(true);
      expect(kinds.has('action')).toBe(true);
    });
  });
});
