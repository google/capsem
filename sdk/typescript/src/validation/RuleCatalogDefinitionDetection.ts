// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RuleCatalogDefinitionDetection} from "../models/RuleCatalogDefinitionDetection.js";
import {EnvironmentDetectionRuleSchema} from "./EnvironmentDetectionRule.js";

export const RuleCatalogDefinitionDetectionSchema: z.ZodType<RuleCatalogDefinitionDetection> = z.object({
  "kind": z.literal("detection"),
  "rule": z.lazy(() => EnvironmentDetectionRuleSchema),
});
