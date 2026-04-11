import { describe, it, expect } from 'vitest';
import { SettingsModel } from '../models/settings-model';
import { buildMockSettingsResponse } from '../mock-settings';

function loadModel(): SettingsModel {
  return new SettingsModel(buildMockSettingsResponse());
}

describe('Settings export/import', () => {
  describe('exportToJSON', () => {
    it('produces valid JSON with expected structure', () => {
      const model = loadModel();
      const json = model.exportToJSON();
      const parsed = JSON.parse(json);
      expect(parsed.version).toBe('1');
      expect(parsed.exported_at).toBeDefined();
      expect(typeof parsed.settings).toBe('object');
    });

    it('includes all leaf settings', () => {
      const model = loadModel();
      const parsed = JSON.parse(model.exportToJSON());
      const leafCount = model.flatLeaves.length;
      expect(Object.keys(parsed.settings).length).toBe(leafCount);
    });

    it('each entry has value and corp_locked fields', () => {
      const model = loadModel();
      const parsed = JSON.parse(model.exportToJSON());
      for (const entry of Object.values(parsed.settings)) {
        const e = entry as { value: unknown; corp_locked: boolean };
        expect(e).toHaveProperty('value');
        expect(e).toHaveProperty('corp_locked');
        expect(typeof e.corp_locked).toBe('boolean');
      }
    });

    it('preserves complex values (file type)', () => {
      const model = loadModel();
      const parsed = JSON.parse(model.exportToJSON());
      const bashrc = parsed.settings['vm.environment.shell.bashrc'];
      expect(bashrc.value).toHaveProperty('path');
      expect(bashrc.value).toHaveProperty('content');
    });
  });

  describe('importFromJSON', () => {
    it('returns changes for differing values', () => {
      const model = loadModel();
      const importData = JSON.stringify({
        version: '1',
        exported_at: new Date().toISOString(),
        settings: {
          'vm.resources.cpu_count': { value: 8, corp_locked: false },
        },
      });
      const changes = model.importFromJSON(importData);
      expect(changes.size).toBe(1);
      expect(changes.get('vm.resources.cpu_count')).toBe(8);
    });

    it('skips settings whose value matches current', () => {
      const model = loadModel();
      // CPU count defaults to 4 in mock data
      const importData = JSON.stringify({
        version: '1',
        exported_at: new Date().toISOString(),
        settings: {
          'vm.resources.cpu_count': { value: 4, corp_locked: false },
        },
      });
      const changes = model.importFromJSON(importData);
      expect(changes.size).toBe(0);
    });

    it('skips corp-locked settings', () => {
      const model = loadModel();
      // Manually mark a leaf as corp_locked for testing
      const leaf = model.getLeaf('vm.resources.cpu_count');
      if (leaf) (leaf as { corp_locked: boolean }).corp_locked = true;
      const importData = JSON.stringify({
        version: '1',
        exported_at: new Date().toISOString(),
        settings: {
          'vm.resources.cpu_count': { value: 8, corp_locked: false },
        },
      });
      const changes = model.importFromJSON(importData);
      expect(changes.size).toBe(0);
    });

    it('skips unknown setting IDs', () => {
      const model = loadModel();
      const importData = JSON.stringify({
        version: '1',
        exported_at: new Date().toISOString(),
        settings: {
          'nonexistent.setting': { value: 'hello', corp_locked: false },
        },
      });
      const changes = model.importFromJSON(importData);
      expect(changes.size).toBe(0);
    });

    it('accepts raw values (no wrapper object)', () => {
      const model = loadModel();
      const importData = JSON.stringify({
        version: '1',
        exported_at: new Date().toISOString(),
        settings: {
          'vm.resources.cpu_count': 8,
        },
      });
      const changes = model.importFromJSON(importData);
      expect(changes.get('vm.resources.cpu_count')).toBe(8);
    });

    it('throws on invalid JSON', () => {
      const model = loadModel();
      expect(() => model.importFromJSON('not json')).toThrow('Invalid JSON');
    });

    it('throws on wrong version', () => {
      const model = loadModel();
      const importData = JSON.stringify({ version: '99', settings: {} });
      expect(() => model.importFromJSON(importData)).toThrow('Unsupported settings version: 99');
    });

    it('throws on missing version', () => {
      const model = loadModel();
      const importData = JSON.stringify({ settings: {} });
      expect(() => model.importFromJSON(importData)).toThrow('Unsupported settings version: missing');
    });

    it('throws on missing settings object', () => {
      const model = loadModel();
      const importData = JSON.stringify({ version: '1' });
      expect(() => model.importFromJSON(importData)).toThrow('missing settings object');
    });

    it('throws on non-object input', () => {
      const model = loadModel();
      expect(() => model.importFromJSON('"just a string"')).toThrow('expected an object');
    });

    it('throws on array input', () => {
      const model = loadModel();
      expect(() => model.importFromJSON('[]')).toThrow('expected an object');
    });
  });

  describe('round-trip', () => {
    it('export then import produces no changes', () => {
      const model = loadModel();
      const json = model.exportToJSON();
      const changes = model.importFromJSON(json);
      expect(changes.size).toBe(0);
    });

    it('export, modify one value, import produces one change', () => {
      const model = loadModel();
      const parsed = JSON.parse(model.exportToJSON());
      parsed.settings['vm.resources.cpu_count'].value = 8;
      const changes = model.importFromJSON(JSON.stringify(parsed));
      expect(changes.size).toBe(1);
      expect(changes.get('vm.resources.cpu_count')).toBe(8);
    });
  });
});
