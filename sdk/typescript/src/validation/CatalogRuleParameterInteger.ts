// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CatalogRuleParameterInteger} from "../models/CatalogRuleParameterInteger.js";

export const CatalogRuleParameterIntegerSchema: z.ZodType<CatalogRuleParameterInteger> = z.object({
  "default": z.int(),
  "max": z.int(),
  "min": z.int(),
  "type": z.literal("integer"),
});
