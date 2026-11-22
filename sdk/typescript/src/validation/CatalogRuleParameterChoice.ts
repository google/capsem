// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CatalogRuleParameterChoice} from "../models/CatalogRuleParameterChoice.js";

export const CatalogRuleParameterChoiceSchema: z.ZodType<CatalogRuleParameterChoice> = z.object({
  "choices": z.array(z.string()),
  "default": z.string(),
  "type": z.literal("choice"),
});
