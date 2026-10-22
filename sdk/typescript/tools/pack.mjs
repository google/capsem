// Preserve the package manager selected by the invoking pnpm script, even when
// the clean-consumer fixture changes HOME or removes Corepack's cache.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {basename, isAbsolute} from 'node:path';
import process from 'node:process';

/**
 * @param {string} project
 * @param {string} destination
 * @param {NodeJS.ProcessEnv} environment
 */
export function packPackage(project, destination, environment = process.env) {
  const manager = process.env.npm_execpath;
  assert.ok(manager && isAbsolute(manager) && /^pnpm(?:\.[cm]?js|\.exe)?$/i.test(basename(manager)),
    'run packaging through pnpm run to retain the selected package manager');
  const javascript = /\.[cm]?js$/i.test(manager);
  const args = ['pack', '--config.ignore-scripts=true', '--pack-destination', destination];
  execFileSync(javascript ? process.execPath : manager, javascript ? [manager, ...args] : args, {
    cwd: project, stdio: 'pipe', timeout: 15_000, env: environment,
  });
}
