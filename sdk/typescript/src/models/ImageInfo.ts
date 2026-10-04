// Generated from Capsem OpenAPI. Do not edit.

import type { ImageCacheState } from "./ImageCacheState.js";

export interface ImageInfo {
  "architectures": Array<string>;
  "cached": ImageCacheState;
  "description": string;
  "image"?: string | null;
  "name": string;
}
