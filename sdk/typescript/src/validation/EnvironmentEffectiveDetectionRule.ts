// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentEffectiveDetectionRule} from "../models/EnvironmentEffectiveDetectionRule.js";
import {DetectionLevelSchema} from "./DetectionLevel.js";
import {EnvironmentRuleSourceSchema} from "./EnvironmentRuleSource.js";

export const EnvironmentEffectiveDetectionRuleSchema: z.ZodType<EnvironmentEffectiveDetectionRule> = z.strictObject({
  "condition": z.string(),
  "editable_key": z.string().nullable(),
  "enabled": z.boolean(),
  "id": z.string(),
  "level": z.lazy(() => DetectionLevelSchema),
  "name": z.string(),
  "source": z.lazy(() => EnvironmentRuleSourceSchema),
});
