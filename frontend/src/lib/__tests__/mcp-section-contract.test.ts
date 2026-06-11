import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const source = readFileSync(
  new URL('../components/settings/McpSection.svelte', import.meta.url),
  'utf8',
);

describe('McpSection route contract', () => {
  it('renders tool permissions with enum metadata and keeps the route-backed selector', () => {
    expect(source).toContain('const PERMISSION_META: Record<ToolPermission');
    expect(source).toContain('allow:');
    expect(source).toContain('ask:');
    expect(source).toContain('block:');
    expect(source).toContain('<meta.icon');
    expect(source).toContain('setToolPermission(tool, event.currentTarget.value as ToolPermission)');
  });

  it('greys disabled servers from server.enabled without inventing another policy path', () => {
    expect(source).toContain("server.enabled ? '' : 'opacity-70 bg-muted/20'");
    expect(source).not.toContain('approved');
  });
});
