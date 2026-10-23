import { ImageCacheState, ImageSurface, type ImageInfo, type ProvisionRequest } from '@capsem/sdk';

export interface ImageActionFailure {
  image: string;
  action: 'prepare' | 'launch';
  message: string;
}

export function retainImageActionFailure(failure: ImageActionFailure | null, images: readonly ImageInfo[], runtimeReady: boolean): ImageActionFailure | null {
  const image = failure?.action === 'prepare' ? images.find(image => image.name === failure.image) : null;
  return image?.image && !imageNeedsDownload(image, runtimeReady) ? null : failure;
}

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

/** One action owns preparation and creation; failure never creates a session. */
export async function startImageSession(image: ImageInfo, runtimeReady: boolean, actions: {
  prepareRuntime: () => Promise<void>;
  pullImage: (reference: string) => Promise<unknown>;
  launch: () => Promise<void>;
}): Promise<void> {
  if (!image.image) throw new Error('This app is unavailable on this computer.');
  if (!runtimeReady) await actions.prepareRuntime();
  if (image.cached !== ImageCacheState.READY) await actions.pullImage(image.image);
  await actions.launch();
}

export function imageHasUpdate(image: ImageInfo): boolean {
  return Boolean(image.image && image.previous_image && image.previous_image !== image.image &&
    (image.cached === ImageCacheState.MISSING || image.cached === ImageCacheState.PARTIAL));
}
