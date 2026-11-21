// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentEffectiveAccessRule} from "../models/EnvironmentEffectiveAccessRule.js";
import {DetectionLevelSchema} from "./DetectionLevel.js";
import {EnvironmentRuleSourceSchema} from "./EnvironmentRuleSource.js";
import {SecurityRuleActionSchema} from "./SecurityRuleAction.js";

export const EnvironmentEffectiveAccessRuleSchema: z.ZodType<EnvironmentEffectiveAccessRule> = z.strictObject({
  "action": z.lazy(() => SecurityRuleActionSchema),
  "condition": z.string(),
  "detection_level": z.union([z.null(), z.lazy(() => DetectionLevelSchema)]),
  "editable_key": z.string().nullable(),
  "enabled": z.boolean(),
  "id": z.string(),
  "locked": z.boolean(),
  "name": z.string(),
  "priority": z.int(),
  "source": z.lazy(() => EnvironmentRuleSourceSchema),
});
