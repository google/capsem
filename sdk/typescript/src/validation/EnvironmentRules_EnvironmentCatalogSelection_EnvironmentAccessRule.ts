// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRule} from "../models/EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRule.js";
import {EnvironmentCatalogSelections_EnvironmentCatalogSelectionSchema} from "./EnvironmentCatalogSelections_EnvironmentCatalogSelection.js";
import {EnvironmentRuleParameterSchema} from "./EnvironmentRuleParameter.js";
import {EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleCustomValueSchema} from "./EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleCustomValue.js";

export const EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleSchema: z.ZodType<EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRule> = z.strictObject({
  "catalog": z.lazy(() => EnvironmentCatalogSelections_EnvironmentCatalogSelectionSchema).exactOptional(),
  "custom": z.record(z.string(), z.lazy(() => EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleCustomValueSchema)).exactOptional(),
  "parameters": z.record(z.string(), z.record(z.string(), z.lazy(() => EnvironmentRuleParameterSchema))).exactOptional(),
});
