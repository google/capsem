// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentCatalogSelections_EnvironmentCatalogSelection } from "./EnvironmentCatalogSelections_EnvironmentCatalogSelection.js";
import type { EnvironmentRuleParameter } from "./EnvironmentRuleParameter.js";
import type { EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRuleCustomValue } from "./EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRuleCustomValue.js";

export interface EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRule {
  "catalog"?: EnvironmentCatalogSelections_EnvironmentCatalogSelection;
  "custom"?: Record<string, EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRuleCustomValue>;
  "parameters"?: Record<string, Record<string, EnvironmentRuleParameter>>;
}
