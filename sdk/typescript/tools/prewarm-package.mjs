// Fetch dependencies before the clean-consumer acceptance enters the sandbox.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {existsSync, readFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import process from 'node:process';

/** @type {unknown} */
const parsed = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
const manifest = /** @type {{dependencies:Record<string,string>,devDependencies:Record<string,string>}} */ (parsed);
const compiler = manifest.devDependencies.typescript;
assert.ok(compiler);
const home = process.env.HOME && existsSync(process.env.HOME) ? process.env.HOME : tmpdir();
for (const [name, version] of Object.entries({...manifest.dependencies, typescript: compiler})) {
  execFileSync('npm', ['cache', 'add', `${name}@${version}`], {
    stdio: 'inherit', timeout: 30_000,
    env: {HOME: home, PATH: process.env.PATH ?? '',
      ...(process.env.NPM_CONFIG_CACHE ? {NPM_CONFIG_CACHE: process.env.NPM_CONFIG_CACHE} : {}),
      ...(process.env.NPM_CONFIG_OFFLINE ? {NPM_CONFIG_OFFLINE: process.env.NPM_CONFIG_OFFLINE} : {})},
  });
}
