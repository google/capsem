// SQL gateway: routes queries to the gateway /inspect endpoint.
import type { QueryResult } from './types';

/** Execute a SELECT query against the session DB. */
export async function queryDb(sql: string, _params?: unknown[]): Promise<QueryResult> {
  const { inspectQuery } = await import('./api');
  const result = await inspectQuery('_active', sql);
  return { columns: result.columns, rows: result.rows.map(r => Object.values(r)) };
}

/** Execute a SELECT query against the main.db (cross-session index). */
export async function queryDbMain(sql: string, _params?: unknown[]): Promise<QueryResult> {
  const { inspectQuery } = await import('./api');
  const result = await inspectQuery('_main', sql);
  return { columns: result.columns, rows: result.rows.map(r => Object.values(r)) };
}

/** Extract the first row as a typed object (column-name keys). */
export function queryOne<T>(result: QueryResult): T | null {
  if (result.rows.length === 0) return null;
  const obj: Record<string, unknown> = {};
  for (let i = 0; i < result.columns.length; i++) {
    obj[result.columns[i]] = result.rows[0][i];
  }
  return obj as T;
}

/** Extract all rows as typed objects (column-name keys). */
export function queryAll<T>(result: QueryResult): T[] {
  return result.rows.map((row) => {
    const obj: Record<string, unknown> = {};
    for (let i = 0; i < result.columns.length; i++) {
      obj[result.columns[i]] = row[i];
    }
    return obj as T;
  });
}
