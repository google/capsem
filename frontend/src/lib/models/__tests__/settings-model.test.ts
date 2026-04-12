import { describe, it, expect } from 'vitest';
import { SettingsModel } from '../settings-model';
import { Widget } from '../settings-enums';
import { buildMockSettingsResponse } from '../../mock-settings';

function loadModel(): SettingsModel {
  return new SettingsModel(buildMockSettingsResponse());
}

describe('SettingsModel', () => {
  describe('tree indexing', () => {
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
      expect(names).toContain('App');
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
