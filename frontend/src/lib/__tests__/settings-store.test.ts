import { describe, it, expect, beforeEach } from 'vitest';
import { settingsStore } from '../stores/settings.svelte';

describe('settingsStore', () => {
  beforeEach(async () => {
    await settingsStore.load();
  });

  it('load() populates model', () => {
    expect(settingsStore.model).not.toBeNull();
  });

  it('sections includes expected groups', () => {
    expect(settingsStore.sections).toContain('App');
    expect(settingsStore.sections).toContain('AI Providers');
    expect(settingsStore.sections).toContain('VM');
  });

  it('starts not dirty', () => {
    expect(settingsStore.isDirty).toBe(false);
  });

  it('stage marks dirty', () => {
    settingsStore.stage('vm.resources.cpu_count', 8);
    expect(settingsStore.isDirty).toBe(true);
  });

  it('save clears dirty', async () => {
    settingsStore.stage('vm.resources.cpu_count', 8);
    expect(settingsStore.isDirty).toBe(true);
    await settingsStore.save();
    expect(settingsStore.isDirty).toBe(false);
  });

  it('discard reloads and clears dirty', async () => {
    settingsStore.stage('vm.resources.cpu_count', 8);
    await settingsStore.discard();
    expect(settingsStore.isDirty).toBe(false);
    expect(settingsStore.model).not.toBeNull();
  });

  it('findLeaf returns leaf by ID', () => {
    const leaf = settingsStore.findLeaf('ai.anthropic.allow');
    expect(leaf).toBeDefined();
    expect(leaf!.setting_type).toBe('bool');
  });

  it('issuesFor returns issues for known ID', () => {
    const issues = settingsStore.issuesFor('ai.anthropic.api_key');
    expect(issues.length).toBeGreaterThan(0);
  });

  it('section finds group by name', () => {
    const sec = settingsStore.section('Security');
    expect(sec).toBeDefined();
    expect(sec!.key).toBe('security');
  });

  it('updateImmediate applies and saves', async () => {
    const before = settingsStore.findLeaf('security.web.allow_read')?.effective_value;
    await settingsStore.updateImmediate('security.web.allow_read', !before);
    const after = settingsStore.findLeaf('security.web.allow_read')?.effective_value;
    expect(after).toBe(!before);
    expect(settingsStore.isDirty).toBe(false);
  });
});
