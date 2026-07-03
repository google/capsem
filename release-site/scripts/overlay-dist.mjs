import { cpSync, existsSync, statSync } from 'node:fs';
import { isAbsolute, resolve } from 'node:path';

const channelDist = process.env.CAPSEM_RELEASE_CHANNEL_DIST;
if (!channelDist) {
  console.log('CAPSEM_RELEASE_CHANNEL_DIST is unset; skipping release-channel overlay.');
  process.exit(0);
}

const source = resolve('dist');
const target = resolveReleaseDist(channelDist);

if (!existsSync(source)) {
  throw new Error(`Astro output does not exist: ${source}`);
}
if (!existsSync(target)) {
  throw new Error(`Release-channel dist does not exist: ${target}`);
}
if (statSync(target).isFile()) {
  console.log(`CAPSEM_RELEASE_CHANNEL_DIST points at a graph fixture file (${target}); skipping release-channel overlay.`);
  process.exit(0);
}

cpSync(source, target, { recursive: true });

function resolveReleaseDist(path) {
  if (isAbsolute(path)) {
    return path;
  }
  const fromCwd = resolve(process.cwd(), path);
  if (existsSync(fromCwd)) {
    return fromCwd;
  }
  return resolve(process.cwd(), '..', path);
}
