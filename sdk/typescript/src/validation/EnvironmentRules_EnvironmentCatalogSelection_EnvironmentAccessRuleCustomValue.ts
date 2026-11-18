// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleCustomValue} from "../models/EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleCustomValue.js";
import {EnvironmentCapabilityDecisionSchema} from "./EnvironmentCapabilityDecision.js";

export const EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleCustomValueSchema: z.ZodType<EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleCustomValue> = z.strictObject({
  "action": z.lazy(() => EnvironmentCapabilityDecisionSchema),
  "enabled": z.boolean().exactOptional(),
  "match": z.string(),
  "name": z.string(),
  "priority": z.int().nullable().exactOptional(),
});
