// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentCatalogSelections_EnvironmentCatalogPin } from "./EnvironmentCatalogSelections_EnvironmentCatalogPin.js";
import type { EnvironmentRuleParameter } from "./EnvironmentRuleParameter.js";
import type { EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleCustomValue } from "./EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleCustomValue.js";

export interface EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRule {
  "catalog"?: EnvironmentCatalogSelections_EnvironmentCatalogPin;
  "custom"?: Record<string, EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleCustomValue>;
  "parameters"?: Record<string, Record<string, EnvironmentRuleParameter>>;
}
