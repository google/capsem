// Generated from Capsem OpenAPI. Do not edit.

import type { ImagePackageInventory } from "./ImagePackageInventory.js";

export interface ImageInventoryResponse {
  "architecture": string;
  "artifact_reference"?: string | null;
  "image": string;
  "inventory"?: null | ImagePackageInventory;
}
