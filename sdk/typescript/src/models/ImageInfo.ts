// Generated from Capsem OpenAPI. Do not edit.

import type { ImageCacheState } from "./ImageCacheState.js";
import type { ImageDownloadProgress } from "./ImageDownloadProgress.js";
import type { ImageSurface } from "./ImageSurface.js";

export interface ImageInfo {
  "architectures": Array<string>;
  "cached": ImageCacheState;
  "description": string;
  "image"?: string | null;
  "name": string;
  "previous_image"?: string | null;
  "progress"?: null | ImageDownloadProgress;
  "surface"?: null | ImageSurface;
}
