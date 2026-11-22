// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CatalogRuleParameterText} from "../models/CatalogRuleParameterText.js";

export const CatalogRuleParameterTextSchema: z.ZodType<CatalogRuleParameterText> = z.object({
  "allow_empty": z.boolean().exactOptional(),
  "default": z.string(),
  "max_bytes": z.int().min(0),
  "type": z.literal("text"),
});
