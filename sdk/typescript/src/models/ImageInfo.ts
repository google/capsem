// Generated from Capsem OpenAPI. Do not edit.

import type { ImageCacheState } from "./ImageCacheState.js";
import type { ImageSurface } from "./ImageSurface.js";

export interface ImageInfo {
  "architectures": Array<string>;
  "cached": ImageCacheState;
  "description": string;
  "image"?: string | null;
  "name": string;
  "surface"?: null | ImageSurface;
}
