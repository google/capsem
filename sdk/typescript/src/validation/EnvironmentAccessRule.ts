// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentAccessRule} from "../models/EnvironmentAccessRule.js";
import {EnvironmentCapabilityDecisionSchema} from "./EnvironmentCapabilityDecision.js";

export const EnvironmentAccessRuleSchema: z.ZodType<EnvironmentAccessRule> = z.strictObject({
  "action": z.lazy(() => EnvironmentCapabilityDecisionSchema),
  "enabled": z.boolean().exactOptional(),
  "match": z.string(),
  "name": z.string(),
  "priority": z.int().nullable().exactOptional(),
});
