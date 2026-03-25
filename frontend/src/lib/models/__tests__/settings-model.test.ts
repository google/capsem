import { describe, it, expect } from 'vitest';
import { SettingsModel } from '../settings-model';
import { SettingType, Widget, SideEffect, ActionKind, defaultWidget } from '../settings-enums';
import type { SettingsResponse, SettingsLeaf, SettingsGroup, SettingsAction, McpServerNode, SettingsNode } from '../../types';

function makeLeaf(overrides: Partial<SettingsLeaf> = {}): SettingsLeaf {
  return {
    kind: 'leaf',
    id: 'test.setting',
    category: 'Test',
    name: 'Test Setting',
    description: 'A test setting',
    setting_type: 'text',
    default_value: '',
    effective_value: '',
    source: 'default',
    modified: null,
    corp_locked: false,
    enabled_by: null,
    enabled: true,
    metadata: {
      domains: [],
      choices: [],
      min: null,
      max: null,
      rules: {},
    },
    ...overrides,
  };
}

function makeGroup(name: string, children: SettingsNode[]): SettingsGroup {
  return {
    kind: 'group',
    key: name.toLowerCase(),
    name,
    enabled: true,
    collapsed: false,
    children,
  };
}

function makeAction(action: string): SettingsAction {
  return {
    kind: 'action',
    key: `test.${action}`,
    name: `Test ${action}`,
    action,
  };
}

function makeMcpServer(key: string): McpServerNode {
  return {
    kind: 'mcp_server',
    key,
    name: key,
    transport: 'stdio',
    command: `/run/${key}`,
    args: [],
    env: {},
    headers: {},
    builtin: false,
    enabled: true,
    source: 'default',
    corp_locked: false,
  };
}

function makeResponse(tree: SettingsNode[] = []): SettingsResponse {
  return { tree, issues: [], presets: [] };
}

describe('SettingsModel', () => {
  describe('constructor and indexes', () => {
    it('builds leaf index from tree', () => {
      const leaf = makeLeaf({ id: 'a.b.c' });
      const model = new SettingsModel(makeResponse([
        makeGroup('Test', [leaf]),
      ]));
      expect(model.getLeaf('a.b.c')).toBeDefined();
      expect(model.getLeaf('nonexistent')).toBeUndefined();
    });

    it('builds MCP index from tree', () => {
      const mcp = makeMcpServer('capsem');
      const model = new SettingsModel(makeResponse([
        makeGroup('MCP', [mcp]),
      ]));
      expect(model.getMcpServer('capsem')).toBeDefined();
      expect(model.mcpServers).toHaveLength(1);
    });

    it('indexes nested leaves', () => {
      const leaf = makeLeaf({ id: 'deep.leaf' });
      const model = new SettingsModel(makeResponse([
        makeGroup('L1', [makeGroup('L2', [leaf])]),
      ]));
      expect(model.getLeaf('deep.leaf')).toBeDefined();
    });
  });

  describe('sections', () => {
    it('returns only top-level groups', () => {
      const model = new SettingsModel(makeResponse([
        makeGroup('AI', []),
        makeGroup('Security', []),
        makeAction('check_update'),
      ]));
      expect(model.sections).toHaveLength(2);
      expect(model.sections.map((s) => s.name)).toEqual(['AI', 'Security']);
    });

    it('section() finds by name', () => {
      const model = new SettingsModel(makeResponse([
        makeGroup('AI', []),
        makeGroup('Security', []),
      ]));
      expect(model.section('Security')?.name).toBe('Security');
      expect(model.section('Nonexistent')).toBeUndefined();
    });
  });

  describe('getGroup', () => {
    it('finds nested groups by name', () => {
      const inner = makeGroup('Shell', []);
      const model = new SettingsModel(makeResponse([
        makeGroup('VM', [makeGroup('Environment', [inner])]),
      ]));
      expect(model.getGroup('Shell')).toBeDefined();
    });
  });

  describe('issues', () => {
    it('filters issues by id', () => {
      const model = new SettingsModel({
        tree: [],
        issues: [
          { id: 'a', severity: 'warning', message: 'msg1' },
          { id: 'b', severity: 'error', message: 'msg2' },
          { id: 'a', severity: 'error', message: 'msg3' },
        ],
        presets: [],
      });
      expect(model.issuesFor('a')).toHaveLength(2);
      expect(model.issuesFor('b')).toHaveLength(1);
      expect(model.issuesFor('c')).toHaveLength(0);
    });
  });

  describe('presets', () => {
    it('activePresetId matches when all settings match', () => {
      const leaf = makeLeaf({
        id: 'security.web.allow_read',
        setting_type: 'bool',
        effective_value: true,
      });
      const model = new SettingsModel({
        tree: [makeGroup('Security', [leaf])],
        issues: [],
        presets: [
          {
            id: 'medium',
            name: 'Medium',
            description: '',
            settings: { 'security.web.allow_read': true },
            mcp: null,
          },
        ],
      });
      expect(model.activePresetId).toBe('medium');
    });

    it('activePresetId is null when no preset matches', () => {
      const leaf = makeLeaf({
        id: 'security.web.allow_read',
        setting_type: 'bool',
        effective_value: false,
      });
      const model = new SettingsModel({
        tree: [makeGroup('Security', [leaf])],
        issues: [],
        presets: [
          {
            id: 'medium',
            name: 'Medium',
            description: '',
            settings: { 'security.web.allow_read': true },
            mcp: null,
          },
        ],
      });
      expect(model.activePresetId).toBeNull();
    });
  });

  describe('needsSetup', () => {
    it('true when no enabled API key', () => {
      const model = new SettingsModel(makeResponse([
        makeGroup('AI', [
          makeLeaf({ id: 'ai.key', setting_type: 'apikey', effective_value: '', enabled: true }),
        ]),
      ]));
      expect(model.needsSetup).toBe(true);
    });

    it('false when an enabled API key is set', () => {
      const model = new SettingsModel(makeResponse([
        makeGroup('AI', [
          makeLeaf({ id: 'ai.key', setting_type: 'apikey', effective_value: 'sk-test', enabled: true }),
        ]),
      ]));
      expect(model.needsSetup).toBe(false);
    });
  });

  describe('enabled / corp locked', () => {
    it('isEnabled returns leaf enabled state', () => {
      const model = new SettingsModel(makeResponse([
        makeGroup('Test', [
          makeLeaf({ id: 'enabled.one', enabled: true }),
          makeLeaf({ id: 'disabled.one', enabled: false }),
        ]),
      ]));
      expect(model.isEnabled('enabled.one')).toBe(true);
      expect(model.isEnabled('disabled.one')).toBe(false);
      expect(model.isEnabled('nonexistent')).toBe(true);
    });

    it('isCorpLocked returns leaf corp_locked state', () => {
      const model = new SettingsModel(makeResponse([
        makeGroup('Test', [
          makeLeaf({ id: 'locked', corp_locked: true }),
          makeLeaf({ id: 'unlocked', corp_locked: false }),
        ]),
      ]));
      expect(model.isCorpLocked('locked')).toBe(true);
      expect(model.isCorpLocked('unlocked')).toBe(false);
    });
  });

  describe('getSideEffect', () => {
    it('returns enum for known side effect', () => {
      const model = new SettingsModel(makeResponse([
        makeGroup('Test', [
          makeLeaf({
            id: 'dark',
            metadata: { domains: [], choices: [], min: null, max: null, rules: {}, side_effect: 'toggle_theme' },
          }),
        ]),
      ]));
      expect(model.getSideEffect('dark')).toBe(SideEffect.ToggleTheme);
    });

    it('returns null for no side effect', () => {
      const model = new SettingsModel(makeResponse([
        makeGroup('Test', [makeLeaf({ id: 'plain' })]),
      ]));
      expect(model.getSideEffect('plain')).toBeNull();
    });
  });

  describe('getWidget', () => {
    it('uses explicit widget from metadata', () => {
      const leaf = makeLeaf({
        setting_type: 'text',
        metadata: { domains: [], choices: [], min: null, max: null, rules: {}, widget: 'domain_chips' },
      });
      const model = new SettingsModel(makeResponse([makeGroup('T', [leaf])]));
      expect(model.getWidget(leaf)).toBe(Widget.DomainChips);
    });

    it('uses deprecated format=domain_list fallback', () => {
      const leaf = makeLeaf({
        setting_type: 'text',
        metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' },
      });
      const model = new SettingsModel(makeResponse([makeGroup('T', [leaf])]));
      expect(model.getWidget(leaf)).toBe(Widget.DomainChips);
    });

    it('uses select for text with choices', () => {
      const leaf = makeLeaf({
        setting_type: 'text',
        metadata: { domains: [], choices: ['a', 'b'], min: null, max: null, rules: {} },
      });
      const model = new SettingsModel(makeResponse([makeGroup('T', [leaf])]));
      expect(model.getWidget(leaf)).toBe(Widget.Select);
    });

    it('falls back to default widget for type', () => {
      const leaf = makeLeaf({ setting_type: 'bool' });
      const model = new SettingsModel(makeResponse([makeGroup('T', [leaf])]));
      expect(model.getWidget(leaf)).toBe(Widget.Toggle);
    });
  });

  describe('defaultWidget', () => {
    it('maps all types without throwing', () => {
      for (const t of Object.values(SettingType)) {
        expect(() => defaultWidget(t)).not.toThrow();
      }
    });

    it('bool -> Toggle', () => expect(defaultWidget(SettingType.Bool)).toBe(Widget.Toggle));
    it('number -> NumberInput', () => expect(defaultWidget(SettingType.Number)).toBe(Widget.NumberInput));
    it('apikey -> PasswordInput', () => expect(defaultWidget(SettingType.ApiKey)).toBe(Widget.PasswordInput));
    it('file -> FileEditor', () => expect(defaultWidget(SettingType.File)).toBe(Widget.FileEditor));
    it('string_list -> StringChips', () => expect(defaultWidget(SettingType.StringList)).toBe(Widget.StringChips));
    it('text -> TextInput', () => expect(defaultWidget(SettingType.Text)).toBe(Widget.TextInput));
  });

  describe('pending changes', () => {
    it('starts not dirty', () => {
      const model = new SettingsModel(makeResponse([]));
      expect(model.isDirty).toBe(false);
      expect(model.pendingChanges.size).toBe(0);
    });

    it('stage makes dirty', () => {
      const model = new SettingsModel(makeResponse([]));
      model.stage('a', 'value');
      expect(model.isDirty).toBe(true);
      expect(model.pendingChanges.get('a')).toBe('value');
    });

    it('unstage removes change', () => {
      const model = new SettingsModel(makeResponse([]));
      model.stage('a', 'value');
      model.unstage('a');
      expect(model.isDirty).toBe(false);
    });

    it('clearPending removes all', () => {
      const model = new SettingsModel(makeResponse([]));
      model.stage('a', 'v1');
      model.stage('b', 'v2');
      model.clearPending();
      expect(model.isDirty).toBe(false);
    });

    it('getPendingAsRecord converts to object', () => {
      const model = new SettingsModel(makeResponse([]));
      model.stage('x', true);
      model.stage('y', 42);
      const rec = model.getPendingAsRecord();
      expect(rec).toEqual({ x: true, y: 42 });
    });
  });
});
