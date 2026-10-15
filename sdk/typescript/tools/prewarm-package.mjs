// Fetch dependencies before the clean-consumer acceptance enters the sandbox.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdtempSync, readFileSync, rmSync} from 'node:fs';
import {homedir, tmpdir} from 'node:os';
import {join} from 'node:path';
import process from 'node:process';

/** @type {unknown} */
const parsed = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
const manifest = /** @type {{dependencies:Record<string,string>,devDependencies:Record<string,string>}} */ (parsed);
const compiler = manifest.devDependencies.typescript;
assert.ok(compiler);
const cache = process.env.NPM_CONFIG_CACHE ?? join(homedir(), '.npm');
const home = mkdtempSync(join(tmpdir(), 'capsem-sdk-prewarm-home-'));
try {
  for (const [name, version] of Object.entries({...manifest.dependencies, typescript: compiler})) {
    execFileSync('npm', ['cache', 'add', `${name}@${version}`], {
      stdio: 'inherit', timeout: 30_000,
      env: {PATH: process.env.PATH ?? '', HOME: home, NPM_CONFIG_CACHE: cache,
        ...(process.env.NPM_CONFIG_OFFLINE ? {NPM_CONFIG_OFFLINE: process.env.NPM_CONFIG_OFFLINE} : {})},
    });
  }
} finally {
  rmSync(home, {recursive: true, force: true});
}
