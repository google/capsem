// Generated from Capsem OpenAPI. Do not edit.

import type { CatalogEntryMetadata } from "./CatalogEntryMetadata.js";
import type { EnvironmentCatalogPin } from "./EnvironmentCatalogPin.js";

export interface SkillCatalogDetailResponse {
  "files": Record<string, string>;
  "metadata": CatalogEntryMetadata;
  "selection": EnvironmentCatalogPin;
}
