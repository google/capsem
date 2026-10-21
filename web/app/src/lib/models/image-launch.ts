import { ImageCacheState, type ImageInfo, type ProvisionRequest } from '@capsem/sdk';

export function imageNeedsDownload(image: ImageInfo, runtimeReady: boolean): boolean {
  return !runtimeReady || image.cached !== ImageCacheState.READY;
}

export function imageLaunchRequest(image: ImageInfo): ProvisionRequest {
  if (!image.image) throw new Error('This app is unavailable on this computer.');
  return { persistent: true, container: { image: image.image, args: [], env: {}, attach: false } };
}
