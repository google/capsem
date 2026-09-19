import {readFileSync} from 'node:fs';
import {expect, it} from 'vitest';

// The live lane runs only in ironbank, so a wrong comparison there surfaces
// an hour into a release. `vm.exec()` returns ExecOutput objects; comparing
// one to a string (`assert.equal(result.stdout, 'text')`) always throws, and
// checkJs cannot see it through assert's `unknown` parameters. Every output
// comparison goes through the typed helper instead.
it('live acceptance compares exec output only through the typed helper', () => {
  const script = readFileSync(new URL('../tools/live-acceptance.mjs', import.meta.url), 'utf8');
  const untyped = script.split('\n').filter(line => /\bassert(?!Output\()[\w.]*\(.*\.std(out|err)\b(?!\.data)/.test(line));
  expect(untyped).toEqual([]);
  expect(script).toMatch(/function assertOutput\(/);
});
