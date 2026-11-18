// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRule} from "../models/EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRule.js";
import {EnvironmentCatalogSelections_EnvironmentCatalogSelectionSchema} from "./EnvironmentCatalogSelections_EnvironmentCatalogSelection.js";
import {EnvironmentRuleParameterSchema} from "./EnvironmentRuleParameter.js";
import {EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRuleCustomValueSchema} from "./EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRuleCustomValue.js";

export const EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRuleSchema: z.ZodType<EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRule> = z.strictObject({
  "catalog": z.lazy(() => EnvironmentCatalogSelections_EnvironmentCatalogSelectionSchema).exactOptional(),
  "custom": z.record(z.string(), z.lazy(() => EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRuleCustomValueSchema)).exactOptional(),
  "parameters": z.record(z.string(), z.record(z.string(), z.lazy(() => EnvironmentRuleParameterSchema))).exactOptional(),
});
