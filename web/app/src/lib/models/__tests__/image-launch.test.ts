import { expect, it } from 'vitest';
import { ImageCacheState, ImageSurface, type ImageInfo } from '@capsem/sdk';
import { imageNeedsDownload, imageLaunchRequest, imageInterfaceLabel } from '../image-launch';

const image: ImageInfo = { name: 'codex-cli', description: 'Codex', architectures: ['arm64'], cached: ImageCacheState.READY, image: `registry.example/codex@sha256:${'a'.repeat(64)}` };

it('offers Launch only when both app and runtime are ready', () => {
  expect(imageNeedsDownload(image, true)).toBe(false);
  expect(imageNeedsDownload(image, false)).toBe(true);
  for (const cached of [ImageCacheState.MISSING, ImageCacheState.PARTIAL, ImageCacheState.UNKNOWN]) {
    expect(imageNeedsDownload({ ...image, cached }, true)).toBe(true);
  }
});

it('labels declared interfaces and never guesses from the app name', () => {
  expect(imageInterfaceLabel({ ...image, surface: ImageSurface.XPRA })).toBe('Graphical');
  expect(imageInterfaceLabel({ ...image, surface: ImageSurface.TERMINAL })).toBe('Terminal');
  expect(imageInterfaceLabel({ ...image, name: 'claude-desktop' })).toBeNull();
});

it('launches the chosen immutable app with service defaults and rejects unavailable apps', () => {
  expect(imageLaunchRequest(image)).toEqual({ persistent: true, container: { image: image.image, args: [], env: {}, attach: false } });
  expect(() => imageLaunchRequest({ ...image, image: null })).toThrow('unavailable');
});
