import { ImageCacheState, ImageSurface, type ImageInfo, type ProvisionRequest } from '@capsem/sdk';

export function imageInterfaceLabel(image: ImageInfo): string | null {
  switch (image.surface) {
    case ImageSurface.TERMINAL: return 'Terminal';
    case ImageSurface.XPRA: return 'Graphical';
    default: return null;
  }
}

export function imageNeedsDownload(image: ImageInfo, runtimeReady: boolean): boolean {
  return !runtimeReady || image.cached !== ImageCacheState.READY;
}

export function imageLaunchRequest(image: ImageInfo): ProvisionRequest {
  if (!image.image) throw new Error('This app is unavailable on this computer.');
  return { persistent: true, container: { image: image.image, args: [], env: {}, attach: false } };
}
