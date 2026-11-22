// Generated from Capsem OpenAPI. Do not edit.

import type { CatalogEntryMetadata } from "./CatalogEntryMetadata.js";
import type { EnvironmentCatalogPin } from "./EnvironmentCatalogPin.js";
import type { EnvironmentSkillAvailability } from "./EnvironmentSkillAvailability.js";

export interface EnvironmentSkillSummary {
  "availability": EnvironmentSkillAvailability;
  "metadata"?: null | CatalogEntryMetadata;
  "selection": EnvironmentCatalogPin;
}
