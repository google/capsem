import {createRequire} from 'node:module';
import {expect, it} from 'vitest';

it('bounds glob preprocessing in installed lint tooling (GHSA-g3hv-75wg-m249)', () => {
  const load = createRequire(import.meta.url);
  const lint = createRequire(load.resolve('eslint/package.json'));
  const {Minimatch} = lint('minimatch') as {
    Minimatch: new (pattern: string, options: {optimizationLevel: number; braceExpandMax?: number}) => {
      set: unknown[];
      match(path: string): boolean;
    };
  };
  // The vulnerable implementation expands this to 256 patterns despite the
  // explicit limit. Check the bound itself, without a machine-speed deadline.
  const bounded = new Minimatch(Array<string>(8).fill('**/../a/b').join('/'), {
    optimizationLevel: 2, braceExpandMax: 64,
  });
  expect(bounded.set).toHaveLength(64);
  const ordinary = new Minimatch('**/../a/b/**/../c/d', {optimizationLevel: 2});
  expect(ordinary.set).toHaveLength(4);
  expect(ordinary.match('../a/b/../c/d')).toBe(true);
  expect(ordinary.match('x/a/b/y/c/d')).toBe(true);
});
