// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentCapabilityDecision } from "./EnvironmentCapabilityDecision.js";

export interface EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleCustomValue {
  "action": EnvironmentCapabilityDecision;
  "enabled"?: boolean;
  "match": string;
  "name": string;
  "priority"?: number | null;
}
