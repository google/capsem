// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CatalogCompatibility} from "../models/CatalogCompatibility.js";

export const CatalogCompatibilitySchema: z.ZodType<CatalogCompatibility> = z.strictObject({
  "min_runtime_contract": z.int().min(0),
});
