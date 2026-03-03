// SQL gateway: routes queries to Tauri `query_db` command or mock sql.js.
import { isMock } from './mock';
import type { QueryResult } from './types';

/** Execute a SELECT query against the session DB. */
export async function queryDb(sql: string, params?: unknown[]): Promise<QueryResult> {
  if (isMock) {
    const { queryFixture } = await import('./mock');
    return queryFixture(sql, params);
  }
  const { invoke } = await import('@tauri-apps/api/core');
  const raw = await invoke<string>('query_db', {
    sql,
    db: 'session',
    params: params ?? [],
  });
  return JSON.parse(raw);
}

/** Execute a SELECT query against the main.db (cross-session index). */
export async function queryDbMain(sql: string, params?: unknown[]): Promise<QueryResult> {
  if (isMock) {
    const { queryFixtureMain } = await import('./mock');
    return queryFixtureMain(sql, params);
  }
  const { invoke } = await import('@tauri-apps/api/core');
  const raw = await invoke<string>('query_db', {
    sql,
    db: 'main',
    params: params ?? [],
  });
  return JSON.parse(raw);
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
