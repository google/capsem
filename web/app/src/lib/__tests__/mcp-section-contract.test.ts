import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const source = readFileSync(
  new URL('../components/settings/McpSection.svelte', import.meta.url),
  'utf8',
);

describe('McpSection route contract', () => {
  it('renders tool permissions with enum metadata and keeps the route-backed selector', () => {
    expect(source).toContain('const PERMISSIONS: { value: ToolPermission');
    expect(source).toContain('{#each PERMISSIONS as permission');
    expect(source).toContain('const PERMISSION_META: Record<ToolPermission');
    expect(source).toContain('allow:');
    expect(source).toContain('ask:');
    expect(source).toContain('block:');
    expect(source).toContain('<meta.icon');
    expect(source).not.toContain('<option value="allow">Allow</option>');
    expect(source).toContain('setToolPermission(tool, event.currentTarget.value as ToolPermission)');
  });

  it('renders the default MCP permission as the same route-backed rule selector', () => {
    expect(source).toContain('let defaultPermission = $derived(mcpStore.defaultPermission)');
    expect(source).toContain('Default MCP permission');
    expect(source).toContain("defaultPermission.rule_id ?? 'default.mcp'");
    expect(source).toContain('mcpStore.setDefaultPermission(action)');
    expect(source).toContain('setDefaultPermission(event.currentTarget.value as ToolPermission)');
  });

  it('greys disabled servers from server.enabled without inventing another policy path', () => {
    expect(source).toContain("server.enabled ? '' : 'opacity-70 bg-muted/20'");
    expect(source).not.toContain('approved');
  });
});
