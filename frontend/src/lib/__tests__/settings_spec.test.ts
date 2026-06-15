/**
 * Cross-language conformance test for the settings schema.
 *
 * Parses the same golden fixture used by Python and Rust tests.
 * Uses local test interfaces matching the new two-node schema
 * (GroupNode + SettingNode), not the live app's 4-variant types.
 */

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'fs';
import { resolve } from 'path';

// ---------------------------------------------------------------------------
// Test-only interfaces (new two-node schema)
// ---------------------------------------------------------------------------

interface TestMetadata {
  domains: string[];
  choices: string[];
  min?: number | null;
  max?: number | null;
  rules: Record<string, unknown>;
  env_vars?: string[];
  collapsed?: boolean;
  format?: string | null;
  docs_url?: string | null;
  prefix?: string | null;
  filetype?: string | null;
  widget?: string | null;
  side_effect?: string | null;
  hidden: boolean;
  builtin: boolean;
  mask?: boolean;
  validator?: string | null;
  action?: string | null;
  origin?: string | null;
  transport?: string | null;
  command?: string | null;
  url?: string | null;
  args?: string[];
  env?: Record<string, string>;
  headers?: Record<string, string>;
}

interface TestSettingNode {
  kind: 'setting';
  key: string;
  name: string;
  description: string;
  setting_type: string;
  default_value?: unknown;
  effective_value?: unknown;
  source?: string;
  modified?: string | null;
  corp_locked?: boolean;
  enabled_by?: string | null;
  enabled?: boolean;
  collapsed?: boolean;
  metadata: TestMetadata;
  history?: unknown[];
}

interface TestGroupNode {
  kind: 'group';
  key: string;
  name: string;
  description?: string | null;
  enabled_by?: string | null;
  enabled?: boolean;
  collapsed: boolean;
  children: TestNode[];
}

type TestNode = TestGroupNode | TestSettingNode;

interface ExpectedLeaf {
  key: string;
  name: string;
  setting_type: string;
  enabled_by: string | null;
}

interface Expected {
  total_settings: number;
  by_type: Record<string, number>;
  group_count: number;
  settings: ExpectedLeaf[];
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const FIXTURE_DIR = resolve(__dirname, '../../../../tests/settings_spec');

const golden: { settings: TestNode[] } = JSON.parse(
  readFileSync(resolve(FIXTURE_DIR, 'golden.json'), 'utf-8'),
);

const expected: Expected = JSON.parse(
  readFileSync(resolve(FIXTURE_DIR, 'expected.json'), 'utf-8'),
);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function extractSettings(nodes: TestNode[]): TestSettingNode[] {
  const settings: TestSettingNode[] = [];
  for (const node of nodes) {
    if (node.kind === 'setting') {
      settings.push(node);
    } else if (node.kind === 'group') {
      settings.push(...extractSettings(node.children));
    }
  }
  return settings;
}

function countGroups(nodes: TestNode[]): number {
  let count = 0;
  for (const node of nodes) {
    if (node.kind === 'group') {
      count += 1;
      count += countGroups(node.children);
    }
  }
  return count;
}

function countByType(settings: TestSettingNode[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const s of settings) {
    counts[s.setting_type] = (counts[s.setting_type] ?? 0) + 1;
  }
  return counts;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('settings_spec conformance', () => {
  it('golden.json parses successfully', () => {
    expect(golden.settings.length).toBeGreaterThan(0);
  });

  it('total setting count matches expected', () => {
    const settings = extractSettings(golden.settings);
    expect(settings.length).toBe(expected.total_settings);
  });

  it('per-type counts match expected', () => {
    const settings = extractSettings(golden.settings);
    const counts = countByType(settings);
    expect(counts).toEqual(expected.by_type);
  });

  it('group count matches expected', () => {
    expect(countGroups(golden.settings)).toBe(expected.group_count);
  });

  it('setting fields match expected', () => {
    const settings = extractSettings(golden.settings);
    const byKey = new Map(settings.map((s) => [s.key, s]));

    for (const exp of expected.settings) {
      const actual = byKey.get(exp.key);
      expect(actual).toBeDefined();
      expect(actual!.name).toBe(exp.name);
      expect(actual!.setting_type).toBe(exp.setting_type);
      expect(actual!.enabled_by ?? null).toBe(exp.enabled_by);
    }
  });

  it('only app/preference setting types are present in the golden fixture', () => {
    const expectedTypes = new Set([
      'text',
      'number',
      'url',
      'email',
      'bool',
      'kv_map',
      'string_list',
      'int_list',
      'float_list',
      'action',
    ]);
    const settings = extractSettings(golden.settings);
    const types = new Set(settings.map((s) => s.setting_type));
    expect(types).toEqual(expectedTypes);
    for (const forbidden of ['apikey', 'file']) {
      expect(types.has(forbidden)).toBe(false);
    }
  });

  it('action settings have metadata.action', () => {
    const settings = extractSettings(golden.settings);
    const actions = settings.filter((s) => s.setting_type === 'action');
    expect(actions.length).toBeGreaterThanOrEqual(1);
    for (const a of actions) {
      expect(a.metadata.action).toBeDefined();
      expect(a.metadata.action).not.toBeNull();
    }
  });

  it('does not carry profile MCP tools in settings', () => {
    const settings = extractSettings(golden.settings);
    const tools = settings.filter((s) => s.setting_type === 'mcp_tool');
    expect(tools).toHaveLength(0);
  });

  it('does not carry profile/provider file payloads in settings', () => {
    const settings = extractSettings(golden.settings);
    const files = settings.filter((s) => s.setting_type === 'file');
    expect(files).toEqual([]);
    expect(settings.some((s) => s.key.includes('provider'))).toBe(false);
    expect(settings.some((s) => s.key.includes('credential'))).toBe(false);
  });

  it('hidden setting exists', () => {
    const settings = extractSettings(golden.settings);
    expect(settings.some((s) => s.metadata.hidden)).toBe(true);
  });

  it('does not use builtin metadata for profile-owned state', () => {
    const settings = extractSettings(golden.settings);
    expect(settings.some((s) => s.metadata.builtin)).toBe(false);
  });

  it('does not use settings enabled_by to model profile/provider state', () => {
    const settings = extractSettings(golden.settings);
    const withParent = settings.filter((s) => s.enabled_by);
    expect(withParent).toEqual([]);
  });

  it('does not expose AI/provider groups through settings', () => {
    const aiGroup = golden.settings.find(
      (n) => n.kind === 'group' && n.key === 'test_ai',
    ) as TestGroupNode | undefined;
    expect(aiGroup).toBeUndefined();
  });

  it('user-modified setting has source and modified', () => {
    const settings = extractSettings(golden.settings);
    const theme = settings.find((s) => s.key === 'test_appearance.theme');
    expect(theme).toBeDefined();
    expect(theme!.source).toBe('user');
    expect(theme!.modified).toBeDefined();
    expect(theme!.modified).not.toBeNull();
  });
});
