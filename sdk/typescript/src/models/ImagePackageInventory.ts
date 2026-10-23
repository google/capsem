// Generated from Capsem OpenAPI. Do not edit.

import type { ImagePackage } from "./ImagePackage.js";
import type { ImageTool } from "./ImageTool.js";

export interface ImagePackageInventory {
  "architecture": string;
  "packages": Array<ImagePackage>;
  "schema": string;
  "subject": string;
  "tools": Array<ImageTool>;
}
