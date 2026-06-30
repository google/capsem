import { cpSync, existsSync } from 'node:fs';
import { resolve } from 'node:path';

const channelDist = process.env.CAPSEM_RELEASE_CHANNEL_DIST;
if (!channelDist) {
  throw new Error('CAPSEM_RELEASE_CHANNEL_DIST must point at the generated release-channel dist');
}

const source = resolve('dist');
const target = resolve(channelDist);

if (!existsSync(source)) {
  throw new Error(`Astro output does not exist: ${source}`);
}
if (!existsSync(target)) {
  throw new Error(`Release-channel dist does not exist: ${target}`);
}

cpSync(source, target, { recursive: true });
