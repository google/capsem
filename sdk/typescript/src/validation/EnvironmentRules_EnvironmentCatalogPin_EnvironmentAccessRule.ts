// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRule} from "../models/EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRule.js";
import {EnvironmentCatalogSelections_EnvironmentCatalogPinSchema} from "./EnvironmentCatalogSelections_EnvironmentCatalogPin.js";
import {EnvironmentRuleParameterSchema} from "./EnvironmentRuleParameter.js";
import {EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleCustomValueSchema} from "./EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleCustomValue.js";

export const EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleSchema: z.ZodType<EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRule> = z.strictObject({
  "catalog": z.lazy(() => EnvironmentCatalogSelections_EnvironmentCatalogPinSchema).exactOptional(),
  "custom": z.record(z.string(), z.lazy(() => EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleCustomValueSchema)).exactOptional(),
  "parameters": z.record(z.string(), z.record(z.string(), z.lazy(() => EnvironmentRuleParameterSchema))).exactOptional(),
});
