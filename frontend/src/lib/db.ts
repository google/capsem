// Unified SQL gateway for both session.db and main.db.
// Works identically in Tauri mode (invoke query_db) and mock mode (sql.js fixture).
//
// All SQL queries use ? positional placeholders (rusqlite/sql.js native syntax).
import { invoke } from '@tauri-apps/api/core';
import { isMock, queryFixture, queryFixtureMain } from './mock';
import type { QueryResult } from './types';

/** Execute a raw SELECT query. Returns columnar {columns, rows}. */
export async function queryDb(
  sql: string,
  params?: unknown[],
  db?: 'session' | 'main',
): Promise<QueryResult> {
  if (isMock) {
    if (db === 'main') return queryFixtureMain(sql, params);
    return queryFixture(sql, params);
  }
  const raw = await invoke<string>('query_db', {
    sql,
    db: db ?? null,
    params: params ?? null,
  });
  return JSON.parse(raw) as QueryResult;
}

/** Run SQL and return the first row as a typed object (or null). */
export async function queryOne<T>(
  sql: string,
  params?: unknown[],
  db?: 'session' | 'main',
): Promise<T | null> {
  const qr = await queryDb(sql, params, db);
  if (qr.rows.length === 0) return null;
  const obj: Record<string, unknown> = {};
  for (let i = 0; i < qr.columns.length; i++) {
    obj[qr.columns[i]] = qr.rows[0][i];
  }
  return obj as T;
}

/** Run SQL and return all rows as typed objects. */
export async function queryAll<T>(
  sql: string,
  params?: unknown[],
  db?: 'session' | 'main',
): Promise<T[]> {
  const qr = await queryDb(sql, params, db);
  return qr.rows.map((row) => {
    const obj: Record<string, unknown> = {};
    for (let i = 0; i < qr.columns.length; i++) {
      obj[qr.columns[i]] = row[i];
    }
    return obj as T;
  });
}
