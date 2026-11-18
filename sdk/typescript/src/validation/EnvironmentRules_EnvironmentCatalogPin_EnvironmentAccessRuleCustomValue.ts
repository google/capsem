// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleCustomValue} from "../models/EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleCustomValue.js";
import {EnvironmentCapabilityDecisionSchema} from "./EnvironmentCapabilityDecision.js";

export const EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleCustomValueSchema: z.ZodType<EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleCustomValue> = z.strictObject({
  "action": z.lazy(() => EnvironmentCapabilityDecisionSchema),
  "enabled": z.boolean().exactOptional(),
  "match": z.string(),
  "name": z.string(),
  "priority": z.int().nullable().exactOptional(),
});
