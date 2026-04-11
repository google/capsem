import { describe, it, expect, beforeEach } from 'vitest';
import { settingsStore } from '../stores/settings.svelte';

describe('settingsStore', () => {
  beforeEach(async () => {
    await settingsStore.load();
  });

  describe('load', () => {
    it('populates model', () => {
      expect(settingsStore.model).not.toBeNull();
    });

    it('sections includes expected groups', () => {
      expect(settingsStore.sections).toContain('App');
      expect(settingsStore.sections).toContain('AI Providers');
      expect(settingsStore.sections).toContain('VM');
    });

    it('tree is non-empty after load', () => {
      expect(settingsStore.tree.length).toBeGreaterThan(0);
    });

    it('issues are populated after load', () => {
      expect(settingsStore.issues.length).toBeGreaterThan(0);
    });

    it('presets are populated after load', () => {
      expect(settingsStore.model!.presets.length).toBeGreaterThan(0);
    });

    it('loading flag is false after load completes', () => {
      expect(settingsStore.loading).toBe(false);
    });

    it('error is null after successful load', () => {
      expect(settingsStore.error).toBeNull();
    });

    it('double load replaces model cleanly', async () => {
      const firstModel = settingsStore.model;
      await settingsStore.load();
      expect(settingsStore.model).not.toBeNull();
      expect(settingsStore.model).not.toBe(firstModel);
    });
  });

  describe('stage', () => {
    it('marks dirty', () => {
      settingsStore.stage('vm.resources.cpu_count', 8);
      expect(settingsStore.isDirty).toBe(true);
    });

    it('starts not dirty', () => {
      expect(settingsStore.isDirty).toBe(false);
    });

    it('staging same value twice keeps one pending change', () => {
      settingsStore.stage('vm.resources.cpu_count', 8);
      settingsStore.stage('vm.resources.cpu_count', 16);
      expect(settingsStore.model!.pendingChanges.size).toBe(1);
      expect(settingsStore.model!.pendingChanges.get('vm.resources.cpu_count')).toBe(16);
    });

    it('staging multiple keys tracks all', () => {
      settingsStore.stage('vm.resources.cpu_count', 8);
      settingsStore.stage('vm.resources.ram_gb', 16);
      settingsStore.stage('security.web.allow_read', true);
      expect(settingsStore.model!.pendingChanges.size).toBe(3);
    });

    it('staging a boolean value works', () => {
      settingsStore.stage('security.web.allow_read', true);
      expect(settingsStore.model!.pendingChanges.get('security.web.allow_read')).toBe(true);
    });

    it('staging a string value works', () => {
      settingsStore.stage('vm.environment.shell.term', 'xterm');
      expect(settingsStore.model!.pendingChanges.get('vm.environment.shell.term')).toBe('xterm');
    });

    it('staging a complex file value works', () => {
      const fileVal = { path: '/root/.bashrc', content: 'echo hi' };
      settingsStore.stage('vm.environment.shell.bashrc', fileVal);
      expect(settingsStore.model!.pendingChanges.get('vm.environment.shell.bashrc')).toEqual(fileVal);
    });
  });

  describe('save', () => {
    it('clears dirty', async () => {
      settingsStore.stage('vm.resources.cpu_count', 8);
      expect(settingsStore.isDirty).toBe(true);
      await settingsStore.save();
      expect(settingsStore.isDirty).toBe(false);
    });

    it('persists staged value into effective_value', async () => {
      settingsStore.stage('vm.resources.cpu_count', 8);
      await settingsStore.save();
      const leaf = settingsStore.findLeaf('vm.resources.cpu_count');
      expect(leaf!.effective_value).toBe(8);
    });

    it('saves multiple staged changes at once', async () => {
      settingsStore.stage('vm.resources.cpu_count', 8);
      settingsStore.stage('vm.resources.ram_gb', 16);
      await settingsStore.save();
      expect(settingsStore.isDirty).toBe(false);
      expect(settingsStore.findLeaf('vm.resources.cpu_count')!.effective_value).toBe(8);
      expect(settingsStore.findLeaf('vm.resources.ram_gb')!.effective_value).toBe(16);
    });

    it('no-op when not dirty', async () => {
      const modelBefore = settingsStore.model;
      await settingsStore.save();
      // Model reference unchanged (save short-circuits)
      expect(settingsStore.model).toBe(modelBefore);
    });

    it('save then stage again makes dirty again', async () => {
      settingsStore.stage('vm.resources.cpu_count', 8);
      await settingsStore.save();
      expect(settingsStore.isDirty).toBe(false);
      settingsStore.stage('vm.resources.cpu_count', 2);
      expect(settingsStore.isDirty).toBe(true);
    });
  });

  describe('discard', () => {
    it('reloads and clears dirty', async () => {
      settingsStore.stage('vm.resources.cpu_count', 8);
      await settingsStore.discard();
      expect(settingsStore.isDirty).toBe(false);
      expect(settingsStore.model).not.toBeNull();
    });

    it('reverts staged value back to default', async () => {
      const original = settingsStore.findLeaf('vm.resources.cpu_count')!.effective_value;
      settingsStore.stage('vm.resources.cpu_count', 99);
      await settingsStore.discard();
      expect(settingsStore.findLeaf('vm.resources.cpu_count')!.effective_value).toBe(original);
    });

    it('discard when not dirty still reloads', async () => {
      await settingsStore.discard();
      expect(settingsStore.model).not.toBeNull();
      expect(settingsStore.isDirty).toBe(false);
    });
  });

  describe('updateImmediate', () => {
    it('applies and saves in one call', async () => {
      const before = settingsStore.findLeaf('security.web.allow_read')?.effective_value;
      await settingsStore.updateImmediate('security.web.allow_read', !before);
      const after = settingsStore.findLeaf('security.web.allow_read')?.effective_value;
      expect(after).toBe(!before);
      expect(settingsStore.isDirty).toBe(false);
    });

    it('does not leave other staged changes', async () => {
      settingsStore.stage('vm.resources.cpu_count', 8);
      await settingsStore.updateImmediate('security.web.allow_read', true);
      // The cpu_count was also saved (updateImmediate calls save)
      expect(settingsStore.isDirty).toBe(false);
    });
  });

  describe('lookup', () => {
    it('findLeaf returns leaf by ID', () => {
      const leaf = settingsStore.findLeaf('ai.anthropic.allow');
      expect(leaf).toBeDefined();
      expect(leaf!.setting_type).toBe('bool');
    });

    it('findLeaf returns undefined for unknown ID', () => {
      expect(settingsStore.findLeaf('does.not.exist')).toBeUndefined();
    });

    it('findGroup returns group by name', () => {
      const g = settingsStore.findGroup('Claude Code');
      expect(g).toBeDefined();
      expect(g!.key).toBe('ai.anthropic.claude');
    });

    it('findGroup returns undefined for unknown name', () => {
      expect(settingsStore.findGroup('Nonexistent')).toBeUndefined();
    });

    it('issuesFor returns issues for known ID', () => {
      const issues = settingsStore.issuesFor('ai.anthropic.api_key');
      expect(issues.length).toBeGreaterThan(0);
    });

    it('issuesFor returns empty for ID without issues', () => {
      expect(settingsStore.issuesFor('vm.resources.cpu_count')).toEqual([]);
    });

    it('section finds group by name', () => {
      const sec = settingsStore.section('Security');
      expect(sec).toBeDefined();
      expect(sec!.key).toBe('security');
    });

    it('section returns undefined for unknown name', () => {
      expect(settingsStore.section('Nonexistent')).toBeUndefined();
    });

    it('needsSetup is true when no API keys set', () => {
      expect(settingsStore.needsSetup).toBe(true);
    });

    it('activePresetId is null when no preset matches', () => {
      expect(settingsStore.activePresetId).toBeNull();
    });
  });

  describe('presets', () => {
    it('applySecurityPreset changes settings', async () => {
      await settingsStore.applySecurityPreset('medium');
      const webRead = settingsStore.findLeaf('security.web.allow_read');
      expect(webRead!.effective_value).toBe(true);
    });

    it('applySecurityPreset clears applying flag', async () => {
      await settingsStore.applySecurityPreset('high');
      expect(settingsStore.applyingPreset).toBeNull();
    });
  });
});
