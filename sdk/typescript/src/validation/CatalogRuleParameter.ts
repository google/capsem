// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CatalogRuleParameter} from "../models/CatalogRuleParameter.js";
import {CatalogRuleParameterBooleanSchema} from "./CatalogRuleParameterBoolean.js";
import {CatalogRuleParameterChoiceSchema} from "./CatalogRuleParameterChoice.js";
import {CatalogRuleParameterIntegerSchema} from "./CatalogRuleParameterInteger.js";
import {CatalogRuleParameterTextSchema} from "./CatalogRuleParameterText.js";

export const CatalogRuleParameterSchema: z.ZodType<CatalogRuleParameter> = z.union([z.lazy(() => CatalogRuleParameterBooleanSchema), z.lazy(() => CatalogRuleParameterIntegerSchema), z.lazy(() => CatalogRuleParameterTextSchema), z.lazy(() => CatalogRuleParameterChoiceSchema)]);
