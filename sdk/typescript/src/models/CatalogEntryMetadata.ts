// Generated from Capsem OpenAPI. Do not edit.

import type { CatalogCompatibility } from "./CatalogCompatibility.js";
import type { CatalogEntryStatus } from "./CatalogEntryStatus.js";

export interface CatalogEntryMetadata {
  "compatibility": CatalogCompatibility;
  "description"?: string;
  "id": string;
  "license": string;
  "name": string;
  "provenance": string;
  "publisher": string;
  "status": CatalogEntryStatus;
  "status_reason"?: string | null;
  "tags"?: Array<string>;
  "version": string;
}
