// Generated from Capsem OpenAPI. Do not edit.

import type { ImageEvidence } from "./ImageEvidence.js";
import type { ImageInterface } from "./ImageInterface.js";

export interface ImageApp {
  "architecture": string;
  "catalog_name": string;
  "category"?: string | null;
  "description": string;
  "display_name": string;
  "evidence"?: Array<ImageEvidence>;
  "icon"?: string | null;
  "image": string;
  "interfaces"?: Array<ImageInterface>;
  "publisher"?: string | null;
  "url"?: string | null;
}
