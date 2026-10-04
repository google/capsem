// Generated from Capsem OpenAPI. Do not edit.

import type { CatalogInfo } from "./CatalogInfo.js";
import type { ImageInfo } from "./ImageInfo.js";

export interface ImageListResponse {
  "catalog"?: null | CatalogInfo;
  "images": Array<ImageInfo>;
}
