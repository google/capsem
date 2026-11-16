import {afterEach, expect, it, vi} from 'vitest';
import {delegatedBearer} from '../src/index.js';

const id = 'fc120782-2272-4b3d-a2e6-a26ee2f8ab61';
const capability = 'af'.repeat(32);
afterEach(() => vi.unstubAllGlobals());

it('constructs delegated authority explicitly without connecting or discovering a gateway', () => {
  const fetch = vi.fn(() => {throw new Error('must not connect');});
  vi.stubGlobal('fetch', fetch);
  expect(delegatedBearer(id, capability)).toBe(`${id}.${capability}`);
  expect(fetch).not.toHaveBeenCalled();
});

it.each([
  ['', capability], [id.toUpperCase(), capability], [id.replace('-4b3d-', '-1b3d-'), capability],
  [id, 'private-input-secret'], [id, 'ff'.repeat(31)], [id, 'FF'.repeat(32)],
  [id, `Bearer ${capability}`], [id, `${id}.${capability}`],
])('rejects malformed delegated input without echoing identity or capability', (accessId, secret) => {
  let message = '';
  try {delegatedBearer(accessId, secret);} catch (error) {
    expect(error).toBeInstanceOf(TypeError);
    message = (error as Error).message;
  }
  expect(message).toMatch(/delegated.*access.*capability/i);
  expect(message).not.toContain(capability);
  expect(message).not.toContain('private-input-secret');
  expect(message).not.toContain(id);
});

it('never turns a hex-shaped operator string into an operator bearer', () => {
  const operatorLike = 'a'.repeat(64);
  const bearer = delegatedBearer(id, operatorLike);
  expect(bearer).toBe(`${id}.${operatorLike}`);
  expect(bearer).not.toBe(operatorLike);
});
