// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentDetectionRule} from "../models/EnvironmentDetectionRule.js";
import {DetectionLevelSchema} from "./DetectionLevel.js";

export const EnvironmentDetectionRuleSchema: z.ZodType<EnvironmentDetectionRule> = z.strictObject({
  "enabled": z.boolean().exactOptional(),
  "level": z.lazy(() => DetectionLevelSchema),
  "match": z.string(),
  "name": z.string(),
});
