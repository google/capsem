import { describe, it, expect } from 'vitest';
import { validateSelectOnly } from '../sql';

describe('validateSelectOnly', () => {
  it('accepts valid SELECT queries', () => {
    expect(validateSelectOnly('SELECT * FROM events')).toBeNull();
    expect(validateSelectOnly('select id, name from users')).toBeNull();
    expect(validateSelectOnly('SELECT COUNT(*) FROM http_requests WHERE status = 200')).toBeNull();
    expect(validateSelectOnly('  SELECT 1  ')).toBeNull();
  });

  it('rejects empty queries', () => {
    expect(validateSelectOnly('')).toBe('Query is empty');
    expect(validateSelectOnly('   ')).toBe('Query is empty');
  });

  it('rejects comment-only queries', () => {
    expect(validateSelectOnly('-- just a comment')).toBe('Query is empty (only comments)');
    expect(validateSelectOnly('/* block comment */')).toBe('Query is empty (only comments)');
  });

  it('rejects non-SELECT queries', () => {
    expect(validateSelectOnly('INSERT INTO events VALUES (1)')).toBe('Only SELECT queries are allowed');
    expect(validateSelectOnly('UPDATE events SET x = 1')).toBe('Only SELECT queries are allowed');
    expect(validateSelectOnly('DELETE FROM events')).toBe('Only SELECT queries are allowed');
    expect(validateSelectOnly('DROP TABLE events')).toBe('Only SELECT queries are allowed');
  });

  it('rejects SELECT with dangerous subqueries', () => {
    expect(validateSelectOnly('SELECT * FROM events; DROP TABLE events')).toBe('Query contains forbidden keyword');
    expect(validateSelectOnly('SELECT * FROM events WHERE id IN (DELETE FROM events)')).toBe('Query contains forbidden keyword');
  });

  it('rejects ATTACH/DETACH/PRAGMA', () => {
    expect(validateSelectOnly("SELECT * FROM events; ATTACH DATABASE 'x' AS y")).toBe('Query contains forbidden keyword');
    expect(validateSelectOnly('PRAGMA table_info(events)')).toBe('Only SELECT queries are allowed');
  });

  it('rejects ALTER/CREATE/TRUNCATE/REPLACE', () => {
    expect(validateSelectOnly('ALTER TABLE events ADD COLUMN x')).toBe('Only SELECT queries are allowed');
    expect(validateSelectOnly('CREATE TABLE evil (id int)')).toBe('Only SELECT queries are allowed');
    expect(validateSelectOnly('TRUNCATE events')).toBe('Only SELECT queries are allowed');
    expect(validateSelectOnly('REPLACE INTO events VALUES (1)')).toBe('Only SELECT queries are allowed');
  });

  it('handles comments before SELECT', () => {
    expect(validateSelectOnly('-- comment\nSELECT * FROM events')).toBeNull();
    expect(validateSelectOnly('/* comment */ SELECT * FROM events')).toBeNull();
  });

  it('is case insensitive', () => {
    expect(validateSelectOnly('select * from events')).toBeNull();
    expect(validateSelectOnly('Select * From Events')).toBeNull();
    expect(validateSelectOnly('insert into events values (1)')).toBe('Only SELECT queries are allowed');
  });
});
