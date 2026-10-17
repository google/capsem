// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CatalogInfo} from "../models/CatalogInfo.js";

export const CatalogInfoSchema: z.ZodType<CatalogInfo> = z.object({
  "channel": z.string(),
  "digest": z.string(),
  "generated_at": z.string().nullable().exactOptional(),
  "reference": z.string(),
});
