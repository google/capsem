import { readdirSync, readFileSync, statSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

function read(relativePath: string): string {
  return readFileSync(new URL(`../${relativePath}`, import.meta.url), 'utf8');
}

function sourceFiles(relativePath = '..'): string[] {
  const root = new URL(`${relativePath}/`, import.meta.url);
  const entries = readdirSync(root, { withFileTypes: true });
  const files: string[] = [];

  for (const entry of entries) {
    const child = `${relativePath}/${entry.name}`;
    if (entry.isDirectory()) {
      if (entry.name === '__tests__') continue;
      files.push(...sourceFiles(child));
      continue;
    }
    if (!entry.isFile()) continue;
    if (!/\.(astro|svelte|ts)$/.test(entry.name)) continue;
    if (entry.name.endsWith('.test.ts')) continue;
    files.push(child);
  }

  return files.sort();
}

function readSource(relativePath: string): string {
  return readFileSync(new URL(`${relativePath}`, import.meta.url), 'utf8');
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

  it('keeps retired release UI strings out of product source', () => {
    const forbidden = [
      'VMs',
      'Customize VM',
      'build 20',
      'response_body_preview',
      'request_body_preview',
      'credential:blake3',
      'API error 404',
      'API error 501'
    ];
    const hits: string[] = [];

    for (const file of sourceFiles()) {
      const stat = statSync(new URL(`${file}`, import.meta.url));
      if (stat.size === 0) continue;
      const source = readSource(file);
      for (const term of forbidden) {
        if (source.includes(term)) {
          hits.push(`${file}: ${term}`);
        }
      }
      const withoutSecurityHeader = source.replaceAll('Content-Security-Policy', '');
      if (withoutSecurityHeader.includes('Policy')) {
        hits.push(`${file}: Policy`);
      }
    }

    expect(hits).toEqual([]);
  });
});
