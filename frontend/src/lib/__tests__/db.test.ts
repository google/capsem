import { describe, it, expect } from 'vitest';

describe('db', () => {
  describe('queryOne', () => {
    it('returns null for empty result', async () => {
      const { queryOne } = await import('../db');
      const result = queryOne({ columns: ['a'], rows: [] });
      expect(result).toBeNull();
    });

    it('extracts first row as typed object', async () => {
      const { queryOne } = await import('../db');
      const result = queryOne<{ name: string; age: number }>({
        columns: ['name', 'age'],
        rows: [['Alice', 30], ['Bob', 25]],
      });
      expect(result).toEqual({ name: 'Alice', age: 30 });
    });
  });

  describe('queryAll', () => {
    it('extracts all rows as typed objects', async () => {
      const { queryAll } = await import('../db');
      const result = queryAll<{ x: number }>({
        columns: ['x'],
        rows: [[1], [2], [3]],
      });
      expect(result).toEqual([{ x: 1 }, { x: 2 }, { x: 3 }]);
    });

    it('returns empty array for empty result', async () => {
      const { queryAll } = await import('../db');
      const result = queryAll({ columns: ['a'], rows: [] });
      expect(result).toEqual([]);
    });
  });
});
