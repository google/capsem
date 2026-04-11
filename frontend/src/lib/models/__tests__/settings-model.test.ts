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

    it('activePresetId returns null when no preset matches', () => {
      const model = loadModel();
      // Default settings don't match any preset exactly
      expect(model.activePresetId).toBeNull();
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
});
