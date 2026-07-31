import {
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  rmSync,
  statSync,
} from 'node:fs';
import { isAbsolute, join, resolve } from 'node:path';

const channelDist = process.env.CAPSEM_RELEASE_CHANNEL_DIST;
if (!channelDist) {
  console.log('CAPSEM_RELEASE_CHANNEL_DIST is unset; skipping release-channel overlay.');
  process.exit(0);
}

// Passed in, not assumed. Hardcoding Astro's default `outDir` couples the
// overlay to a setting the build is free to change, and the failure would be
// silent: an overlay of a directory that no longer exists, or of a stale one.
const source = resolve(process.argv[2] ?? process.env.CAPSEM_RELEASE_ASTRO_DIST ?? 'dist');
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

overlayTree(source, target);

function overlayTree(sourceDir, targetDir) {
  mkdirSync(targetDir, { recursive: true });
  for (const entry of readdirSync(sourceDir, { withFileTypes: true })) {
    const sourcePath = join(sourceDir, entry.name);
    const targetPath = join(targetDir, entry.name);
    if (entry.isDirectory()) {
      if (existsSync(targetPath) && !statSync(targetPath).isDirectory()) {
        rmSync(targetPath, { recursive: true, force: true });
      }
      overlayTree(sourcePath, targetPath);
      continue;
    }
    rmSync(targetPath, { recursive: true, force: true });
    cpSync(sourcePath, targetPath);
  }
}

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
