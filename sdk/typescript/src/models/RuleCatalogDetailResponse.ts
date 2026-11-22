// Generated from Capsem OpenAPI. Do not edit.

import type { CatalogEntryMetadata } from "./CatalogEntryMetadata.js";
import type { CatalogRuleParameter } from "./CatalogRuleParameter.js";
import type { EnvironmentCatalogPin } from "./EnvironmentCatalogPin.js";
import type { RuleCatalogDefinition } from "./RuleCatalogDefinition.js";

export interface RuleCatalogDetailResponse {
  "definition": RuleCatalogDefinition;
  "metadata": CatalogEntryMetadata;
  "parameters": Record<string, CatalogRuleParameter>;
  "selection": EnvironmentCatalogPin;
}
