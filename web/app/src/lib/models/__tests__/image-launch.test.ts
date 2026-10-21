import { expect, it } from 'vitest';
import { ImageCacheState, type ImageInfo } from '@capsem/sdk';
import { imageNeedsDownload, imageLaunchRequest } from '../image-launch';

const image: ImageInfo = { name: 'codex-cli', description: 'Codex', architectures: ['arm64'], cached: ImageCacheState.READY, image: `registry.example/codex@sha256:${'a'.repeat(64)}` };

it('offers Launch only when both app and runtime are ready', () => {
  expect(imageNeedsDownload(image, true)).toBe(false);
  expect(imageNeedsDownload(image, false)).toBe(true);
  for (const cached of [ImageCacheState.MISSING, ImageCacheState.PARTIAL, ImageCacheState.UNKNOWN]) {
    expect(imageNeedsDownload({ ...image, cached }, true)).toBe(true);
  }
});

it('launches the chosen immutable app with service defaults and rejects unavailable apps', () => {
  expect(imageLaunchRequest(image)).toEqual({ persistent: true, container: { image: image.image, args: [], env: {}, attach: false } });
  expect(() => imageLaunchRequest({ ...image, image: null })).toThrow('unavailable');
});
