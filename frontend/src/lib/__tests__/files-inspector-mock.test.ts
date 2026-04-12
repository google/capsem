import { describe, it, expect } from 'vitest';
import {
  mockFileTree, findFileNode,
  mockPresetQueries, mockQueryResults, executeMockQuery,
} from '../mock.ts';
import type { MockFileNode } from '../mock.ts';

// ---------------------------------------------------------------------------
// File tree
// ---------------------------------------------------------------------------

describe('mockFileTree', () => {
  it('has top-level entries', () => {
    expect(mockFileTree.length).toBeGreaterThan(0);
  });

  it('contains both files and directories', () => {
    const types = new Set<string>();
    function walk(nodes: MockFileNode[]) {
      for (const n of nodes) {
        types.add(n.type);
        if (n.children) walk(n.children);
      }
    }
    walk(mockFileTree);
    expect(types.has('file')).toBe(true);
    expect(types.has('directory')).toBe(true);
  });

  it('every node has required fields', () => {
    function walk(nodes: MockFileNode[]) {
      for (const n of nodes) {
        expect(n.name).toBeTruthy();
        expect(n.path).toBeTruthy();
        expect(['file', 'directory']).toContain(n.type);
        if (n.type === 'directory') {
          expect(n.children).toBeDefined();
          walk(n.children!);
        }
        if (n.type === 'file') {
          expect(n.content).toBeDefined();
        }
      }
    }
    walk(mockFileTree);
  });

  it('paths are unique across the tree', () => {
    const paths: string[] = [];
    function walk(nodes: MockFileNode[]) {
      for (const n of nodes) {
        paths.push(n.path);
        if (n.children) walk(n.children);
      }
    }
    walk(mockFileTree);
    expect(new Set(paths).size).toBe(paths.length);
  });

  it('child paths are prefixed by parent path', () => {
    function walk(nodes: MockFileNode[], parentPath?: string) {
      for (const n of nodes) {
        if (parentPath) {
          expect(n.path.startsWith(parentPath + '/')).toBe(true);
        }
        if (n.children) walk(n.children, n.path);
      }
    }
    walk(mockFileTree);
  });
});

describe('findFileNode', () => {
  it('finds files by path', () => {
    const node = findFileNode(mockFileTree, '/workspace/src/main.rs');
    expect(node).toBeDefined();
    expect(node?.name).toBe('main.rs');
    expect(node?.type).toBe('file');
  });

  it('finds directories by path', () => {
    const node = findFileNode(mockFileTree, '/workspace/src');
    expect(node).toBeDefined();
    expect(node?.name).toBe('src');
    expect(node?.type).toBe('directory');
  });

  it('finds nested files', () => {
    const node = findFileNode(mockFileTree, '/workspace/src/utils/config.rs');
    expect(node).toBeDefined();
    expect(node?.name).toBe('config.rs');
  });

  it('returns undefined for non-existent paths', () => {
    expect(findFileNode(mockFileTree, '/workspace/does/not/exist')).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// Inspector / SQL mock data
// ---------------------------------------------------------------------------

describe('mockPresetQueries', () => {
  it('has presets', () => {
    expect(mockPresetQueries.length).toBeGreaterThan(0);
  });

  it('each preset has label and sql', () => {
    for (const p of mockPresetQueries) {
      expect(p.label).toBeTruthy();
      expect(p.sql).toBeTruthy();
      expect(p.sql.toUpperCase()).toContain('SELECT');
    }
  });

  it('each preset has matching results', () => {
    for (const p of mockPresetQueries) {
      expect(mockQueryResults[p.label]).toBeDefined();
      const result = mockQueryResults[p.label];
      expect(result.columns.length).toBeGreaterThan(0);
      expect(result.rows.length).toBeGreaterThan(0);
    }
  });
});

describe('executeMockQuery', () => {
  it('returns preset results for matching SQL', () => {
    const preset = mockPresetQueries[0];
    const result = executeMockQuery(preset.sql);
    expect(result.columns).toEqual(mockQueryResults[preset.label].columns);
    expect(result.rows).toEqual(mockQueryResults[preset.label].rows);
  });

  it('returns generic result for non-matching SELECT', () => {
    const result = executeMockQuery('SELECT 42');
    expect(result.columns).toEqual(['result']);
    expect(result.rows.length).toBe(1);
  });
});
