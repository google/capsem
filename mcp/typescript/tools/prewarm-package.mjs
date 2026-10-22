// Warm npm's complete runtime dependency graph before offline acceptance.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {copyFileSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync} from 'node:fs';
import {homedir, tmpdir} from 'node:os';
import {join} from 'node:path';
import process from 'node:process';
import {fileURLToPath} from 'node:url';
import {packPackage} from '../../../sdk/typescript/tools/pack.mjs';

const project = fileURLToPath(new URL('../', import.meta.url));
const fixture = mkdtempSync(join(tmpdir(), 'capsem-mcp-prewarm-'));
try {
  const archives = [];
  for (const [owner, source] of [['sdk', join(project, '../../sdk/typescript')], ['mcp', project]]) {
    assert.ok(owner && source);
    const selectedDirectory = process.env.CAPSEM_MCP_PACKAGE_ARCHIVES;
    const output = selectedDirectory ?? join(fixture, owner);
    mkdirSync(output, {recursive: true});
    const version = (JSON.parse(readFileSync(join(source, 'package.json'), 'utf8'))).version;
    const sdkDirectory = process.env.CAPSEM_SDK_PACKAGE_ARCHIVES;
    if (owner === 'sdk' && sdkDirectory) {
      const selected = join(sdkDirectory, `capsem-sdk-${version}.tgz`);
      const stat = lstatSync(selected);
      assert.ok(stat.isFile() && !stat.isSymbolicLink(), 'selected SDK archive must be a regular file');
      copyFileSync(selected, join(output, `capsem-sdk-${version}.tgz`));
    } else {
      packPackage(source, output, {...process.env, HOME: fixture});
    }
    const names = readdirSync(output).filter(name => name === `capsem-${owner}-${version}.tgz`);
    assert.equal(names.length, 1);
    assert.ok(names[0]);
    archives.push(join(output, names[0]));
  }
  writeFileSync(join(fixture, 'package.json'), JSON.stringify({name: 'capsem-mcp-prewarm', private: true}));
  execFileSync('npm', ['install', '--omit=dev', '--ignore-scripts', '--no-audit', '--no-fund', ...archives], {
    cwd: fixture, stdio: 'inherit', timeout: 60_000,
    env: {PATH: process.env.PATH ?? '', HOME: fixture,
      NPM_CONFIG_CACHE: process.env.NPM_CONFIG_CACHE ?? join(homedir(), '.npm'),
      ...(process.env.NPM_CONFIG_OFFLINE ? {NPM_CONFIG_OFFLINE: process.env.NPM_CONFIG_OFFLINE} : {})},
  });
} finally {
  rmSync(fixture, {recursive: true, force: true});
}
