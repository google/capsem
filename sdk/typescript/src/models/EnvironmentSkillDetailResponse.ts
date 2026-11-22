// Generated from Capsem OpenAPI. Do not edit.

import type { CatalogEntryMetadata } from "./CatalogEntryMetadata.js";
import type { EnvironmentCatalogPin } from "./EnvironmentCatalogPin.js";
import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";

export interface EnvironmentSkillDetailResponse {
  "discovery_fresh": boolean;
  "files": Record<string, string>;
  "metadata": CatalogEntryMetadata;
  "revision": EnvironmentRevisionReference;
  "selection": EnvironmentCatalogPin;
}
