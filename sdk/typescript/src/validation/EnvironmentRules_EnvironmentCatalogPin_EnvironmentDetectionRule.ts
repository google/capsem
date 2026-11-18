// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRule} from "../models/EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRule.js";
import {EnvironmentCatalogSelections_EnvironmentCatalogPinSchema} from "./EnvironmentCatalogSelections_EnvironmentCatalogPin.js";
import {EnvironmentRuleParameterSchema} from "./EnvironmentRuleParameter.js";
import {EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRuleCustomValueSchema} from "./EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRuleCustomValue.js";

export const EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRuleSchema: z.ZodType<EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRule> = z.strictObject({
  "catalog": z.lazy(() => EnvironmentCatalogSelections_EnvironmentCatalogPinSchema).exactOptional(),
  "custom": z.record(z.string(), z.lazy(() => EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRuleCustomValueSchema)).exactOptional(),
  "parameters": z.record(z.string(), z.record(z.string(), z.lazy(() => EnvironmentRuleParameterSchema))).exactOptional(),
});
