import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

function read(relativePath: string): string {
  return readFileSync(new URL(`../${relativePath}`, import.meta.url), 'utf8');
}

describe('frontend vocabulary contract', () => {
  it('does not expose retired network policy IPC types', () => {
    const types = read('types.ts');

    expect(types).not.toContain('NetworkPolicyResponse');
    expect(types).not.toContain('get_network_policy');
  });

  it('names settings origin as settings source, not policy source', () => {
    const rootTypes = read('types.ts');
    const settingsTypes = read('types/settings.ts');
    const enumTypes = read('models/settings-enums.ts');

    expect(rootTypes).toContain('export type SettingsSource');
    expect(settingsTypes).toContain('export type SettingsSource');
    expect(enumTypes).toContain('export enum SettingsSource');

    expect(rootTypes).not.toContain('PolicySource');
    expect(settingsTypes).not.toContain('PolicySource');
    expect(enumTypes).not.toContain('PolicySource');
  });

  it('does not silently hide retired policy settings sections in the UI', () => {
    const settingsPage = read('components/shell/SettingsPage.svelte');

    expect(settingsPage).toContain("!['ai', 'repository', 'security', 'vm', 'mcp', 'plugins'].includes(s.key)");
    expect(settingsPage).not.toContain("'policy'].includes(s.key)");
  });
});
