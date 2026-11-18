// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRuleCustomValue} from "../models/EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRuleCustomValue.js";
import {DetectionLevelSchema} from "./DetectionLevel.js";

export const EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRuleCustomValueSchema: z.ZodType<EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRuleCustomValue> = z.strictObject({
  "enabled": z.boolean().exactOptional(),
  "level": z.lazy(() => DetectionLevelSchema),
  "match": z.string(),
  "name": z.string(),
});
