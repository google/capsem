import type { ImageInfo, ProvisionRequest } from '@capsem/sdk';

export function sessionCreateRequest(name: string, ramMb: number, cpus: number, image: ImageInfo | null): ProvisionRequest {
  const trimmedName = name.trim();
  if (!trimmedName) throw new Error('Name is required');
  if (image && !image.image) throw new Error('This image is incompatible with the host architecture or runtime');
  return {
    name: trimmedName, persistent: true, ram_mb: ramMb, cpus,
    ...(image ? { container: { image: image.image!, args: [], env: {}, attach: false } } : {}),
  };
}
