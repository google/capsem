import * as api from './operations/index.js';
import type * as models from './models/index.js';
import type {ImageListOptions, ImagePullOptions} from './options.js';
import type {Transport} from './transport.js';
import {registryAccess} from './registry.js';

/** Catalog and prefetch through the service's image admission policy. */
export class Images {
  constructor(private readonly transport: Transport) {}

  async list(options: ImageListOptions = {}): Promise<models.ImageListResponse> {
    return api.listImages(this.transport, {refresh: options.refresh ?? false}, options);
  }

  async pull(image: string, options: ImagePullOptions = {}): Promise<models.ImagePullResponse> {
    if (typeof image !== 'string' || !image.trim()) throw new TypeError('Image must be a nonempty string');
    const registry = registryAccess(options.registry);
    return api.pullImage(this.transport, {body: {
      image, ...(registry === undefined ? {} : {registry}),
    }}, options);
  }
}
