import {rmSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import process from 'node:process';

const root = new URL('../', import.meta.url);
rmSync(new URL('dist/', root), {recursive: true, force: true});
const compiler = fileURLToPath(import.meta.resolve('typescript/bin/tsc'));
const result = spawnSync(process.execPath, [compiler, '-p', 'tsconfig.json'], {
  cwd: fileURLToPath(root), stdio: 'inherit', timeout: 60_000,
});
if (result.error) throw result.error;
process.exit(result.status ?? 1);
