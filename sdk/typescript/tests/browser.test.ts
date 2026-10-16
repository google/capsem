import {Linter} from 'eslint';
import {expect, it} from 'vitest';
import config from '../eslint.config.js';
import {localConnection} from '../src/local-browser.js';

it('requires explicit browser credentials without reading host metadata', () => {
  expect(() => localConnection()).toThrow(/provide both gateway URL and token/);
});

it.each(['fs', 'node:fs', 'child_process', 'node:net'])('source import %s fails the actual browser restriction', specifier => {
  const rules = config.find(entry => entry.files?.includes('src/**/*.ts') && entry.rules?.['no-restricted-imports']);
  if (!rules?.rules) throw new Error('SDK browser import restriction is missing');
  const result = new Linter().verify(`import builtin from '${specifier}';`, {rules: rules.rules});
  expect(result.map(message => message.ruleId)).toContain('no-restricted-imports');
});
