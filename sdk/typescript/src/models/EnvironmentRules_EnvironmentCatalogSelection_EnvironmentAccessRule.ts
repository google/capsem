// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentCatalogSelections_EnvironmentCatalogSelection } from "./EnvironmentCatalogSelections_EnvironmentCatalogSelection.js";
import type { EnvironmentRuleParameter } from "./EnvironmentRuleParameter.js";
import type { EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleCustomValue } from "./EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleCustomValue.js";

export interface EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRule {
  "catalog"?: EnvironmentCatalogSelections_EnvironmentCatalogSelection;
  "custom"?: Record<string, EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleCustomValue>;
  "parameters"?: Record<string, Record<string, EnvironmentRuleParameter>>;
}
