// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentCapabilityDecision } from "./EnvironmentCapabilityDecision.js";

export interface EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleCustomValue {
  "action": EnvironmentCapabilityDecision;
  "enabled"?: boolean;
  "match": string;
  "name": string;
  "priority"?: number | null;
}
