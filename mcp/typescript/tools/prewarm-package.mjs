// Warm npm's complete runtime dependency graph before offline acceptance.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync} from 'node:fs';
import {homedir, tmpdir} from 'node:os';
import {join} from 'node:path';
import process from 'node:process';
import {fileURLToPath} from 'node:url';

const project = fileURLToPath(new URL('../', import.meta.url));
const fixture = mkdtempSync(join(tmpdir(), 'capsem-mcp-prewarm-'));
try {
  const archives = [];
  for (const [owner, source] of [['sdk', join(project, '../../sdk/typescript')], ['mcp', project]]) {
    assert.ok(owner && source);
    const output = join(fixture, owner);
    mkdirSync(output);
    execFileSync('pnpm', ['pack', '--config.ignore-scripts=true', '--pack-destination', output], {
      cwd: source, stdio: 'pipe', timeout: 15_000, env: {...process.env, HOME: fixture},
    });
    const names = readdirSync(output).filter(name => name.endsWith('.tgz'));
    assert.equal(names.length, 1);
    assert.ok(names[0]);
    archives.push(join(output, names[0]));
  }
  writeFileSync(join(fixture, 'package.json'), JSON.stringify({name: 'capsem-mcp-prewarm', private: true}));
  execFileSync('npm', ['install', '--omit=dev', '--ignore-scripts', '--no-audit', '--no-fund', ...archives], {
    cwd: fixture, stdio: 'inherit', timeout: 60_000,
    env: {PATH: process.env.PATH ?? '', HOME: fixture,
      NPM_CONFIG_CACHE: process.env.NPM_CONFIG_CACHE ?? join(homedir(), '.npm')},
  });
} finally {
  rmSync(fixture, {recursive: true, force: true});
}
