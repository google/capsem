// Fetch dependencies before the clean-consumer acceptance enters the sandbox.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {readFileSync} from 'node:fs';

/** @type {unknown} */
const parsed = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
const manifest = /** @type {{dependencies:Record<string,string>,devDependencies:Record<string,string>}} */ (parsed);
const compiler = manifest.devDependencies.typescript;
assert.ok(compiler);
for (const [name, version] of Object.entries({...manifest.dependencies, typescript: compiler})) {
  execFileSync('npm', ['cache', 'add', `${name}@${version}`], {stdio: 'inherit', timeout: 30_000});
}
