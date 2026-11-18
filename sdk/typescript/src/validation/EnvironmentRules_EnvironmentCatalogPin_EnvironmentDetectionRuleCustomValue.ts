// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRuleCustomValue} from "../models/EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRuleCustomValue.js";
import {DetectionLevelSchema} from "./DetectionLevel.js";

export const EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRuleCustomValueSchema: z.ZodType<EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRuleCustomValue> = z.strictObject({
  "enabled": z.boolean().exactOptional(),
  "level": z.lazy(() => DetectionLevelSchema),
  "match": z.string(),
  "name": z.string(),
});
