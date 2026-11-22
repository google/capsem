// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CatalogRuleParameterBoolean} from "../models/CatalogRuleParameterBoolean.js";

export const CatalogRuleParameterBooleanSchema: z.ZodType<CatalogRuleParameterBoolean> = z.object({
  "default": z.boolean(),
  "type": z.literal("boolean"),
});
