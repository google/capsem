import { readFileSync } from 'node:fs';
import { expect, it } from 'vitest';

it('keeps shared destructive colors purple in light and dark themes', () => {
  const source = readFileSync(new URL('../../styles/capsem-theme.css', import.meta.url), 'utf8');
  for (const name of ['destructive', 'destructive-hover', 'destructive-focus']) {
    const tokens = [...source.matchAll(new RegExp(`--${name}: ([^;]+);`, 'g'))];
    expect(tokens).toHaveLength(2);
    for (const token of tokens) expect(token[1]).toMatch(/^var\(--color-purple-/);
  }
});

it('owns readable standalone error text separately from solid button foreground', () => {
  const source = readFileSync(new URL('../../styles/capsem-theme.css', import.meta.url), 'utf8');
  expect(source).toContain('--color-destructive-text: var(--destructive-text);');
  expect([...source.matchAll(/--destructive-text: ([^;]+);/g)].map(match => match[1])).toEqual([
    'var(--color-purple-700)', 'var(--color-purple-300)',
  ]);
});
