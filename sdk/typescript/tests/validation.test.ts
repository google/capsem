import {describe, expect, it} from 'vitest';
import type {z} from 'zod';
import * as generated from '../src/validation/index.js';
import {CredentialEventType, HostLogSource} from '../src/models/index.js';
import {schemas, sample} from './contract.js';

const validators: Record<string, z.ZodType> = generated;

describe.each(Object.entries(schemas))('%s wire contract', (name, schema) => {
  const validator = validators[`${name}Schema`];
  if (!validator) throw new Error(`No validator for ${name}`);
  it.each([false, true])('round trips wire values (all fields: %s)', full => {
    for (let variant = 0; variant < 3; variant++) {
      const value = sample(schema, full, variant);
      expect(validator.parse(value)).toEqual(value);
    }
  });
  it('rejects invalid wire values', () => {
    expect(validator.safeParse(Symbol('invalid')).success).toBe(false);
    if (schema.enum) expect(validator.safeParse('unknown-enum-value').success).toBe(false);
    for (const key of schema.required ?? []) {
      const value = sample(schema) as Record<string, unknown>;
      delete value[key];
      expect(validator.safeParse(value).success, `missing ${key}`).toBe(false);
    }
  });
});

it('preserves exact optional fields and nullable fields', () => {
  expect(generated.UpdateApplyRequestSchema.parse({})).toEqual({});
  expect(() => generated.UpdateApplyRequestSchema.parse({confirmed: null})).toThrow();
  expect(() => generated.UpdateApplyRequestSchema.parse({confirmed: undefined})).toThrow();
  expect(generated.HostLogSourceSchema.parse('service')).toBe(HostLogSource.SERVICE);
  expect(CredentialEventType.HTTP_REQUEST).toBe('http.request');
});

it('rejects coerced or lossy numbers and out-of-contract fields', () => {
  const base = sample(schemas.FileListEntry ?? {}) as Record<string, unknown>;
  for (const size of ['1', -1, 1.5, Infinity, Number.MAX_SAFE_INTEGER + 1]) {
    expect(() => generated.FileListEntrySchema.parse({...base, size})).toThrow();
  }
  const closed = Object.entries(schemas).find(([, schema]) => schema.additionalProperties === false);
  expect(closed).toBeDefined();
  if (!closed) throw new Error('Contract lost its closed object');
  const validator = validators[`${closed[0]}Schema`];
  expect(validator?.safeParse({...sample(closed[1]) as object, unexpected: true}).success).toBe(false);
});
