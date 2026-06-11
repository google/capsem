import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const source = readFileSync(
  new URL('../components/settings/PluginSection.svelte', import.meta.url),
  'utf8',
);

describe('PluginSection route contract', () => {
  it('renders plugin modes from the typed enum with recognizable icons', () => {
    expect(source).toContain('const MODE_META: Record<PluginMode');
    expect(source).toContain('allow:');
    expect(source).toContain('ask:');
    expect(source).toContain('block:');
    expect(source).toContain('rewrite:');
    expect(source).toContain('disable:');
    expect(source).toContain('<modeMeta.icon');
  });

  it('keeps disabled plugins visible but inactive instead of hiding their mode', () => {
    expect(source).toContain("plugin.config.mode === 'disable'");
    expect(source).toContain('bg-muted/20 opacity-70');
    expect(source).toContain("label: 'Disabled'");
  });
});
