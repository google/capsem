import { expect, it, vi } from 'vitest';
import { ImageCacheState, ImageSurface, type ImageInfo } from '@capsem/sdk';
import { imageNeedsDownload, imageLaunchRequest, imageInterfaceLabel, retainImageActionFailure, startImageSession } from '../image-launch';

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

it('clears an obsolete preparation failure once both app and runtime recover', () => {
  const failure = { image: image.name, action: 'prepare' as const, message: 'Download failed' };
  expect(retainImageActionFailure(failure, [image], true)).toBeNull();
  expect(retainImageActionFailure(failure, [image], false)).toBe(failure);
  expect(retainImageActionFailure(failure, [{ ...image, cached: ImageCacheState.PARTIAL }], true)).toBe(failure);
  expect(retainImageActionFailure(failure, [{ ...image, image: null }], true)).toBe(failure);
  expect(retainImageActionFailure(failure, [{ ...image, name: 'another-app' }], true)).toBe(failure);
});

it('preserves a launch failure even when the app is already downloaded', () => {
  const failure = { image: image.name, action: 'launch' as const, message: 'Launch failed' };
  expect(retainImageActionFailure(failure, [image], true)).toBe(failure);
  expect(retainImageActionFailure(null, [image], true)).toBeNull();
});

it('prepares a missing image then launches from the same action', async () => {
  const calls: string[] = [];
  await startImageSession({ ...image, cached: ImageCacheState.MISSING }, false, {
    prepareRuntime: async () => { calls.push('runtime'); },
    pullImage: async reference => { expect(reference).toBe(image.image); calls.push('image'); },
    launch: async () => { calls.push('launch'); },
  });
  expect(calls).toEqual(['runtime', 'image', 'launch']);
});

it('launches a ready app directly and stops on preparation failure', async () => {
  const actions = { prepareRuntime: vi.fn().mockResolvedValue(undefined),
    pullImage: vi.fn().mockResolvedValue(undefined), launch: vi.fn().mockResolvedValue(undefined) };
  await startImageSession(image, true, actions);
  expect(actions.prepareRuntime).not.toHaveBeenCalled();
  expect(actions.pullImage).not.toHaveBeenCalled();
  expect(actions.launch).toHaveBeenCalledTimes(1);
  actions.launch.mockClear();
  actions.pullImage.mockRejectedValueOnce(Error('offline'));
  await expect(startImageSession({ ...image, cached: ImageCacheState.MISSING }, true, actions)).rejects.toThrow('offline');
  expect(actions.launch).not.toHaveBeenCalled();
});
