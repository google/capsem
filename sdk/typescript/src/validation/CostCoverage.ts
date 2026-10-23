// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CostCoverage} from "../models/CostCoverage.js";

export const CostCoverageSchema: z.ZodType<CostCoverage> = z.object({
  "priced_calls": z.int().min(0),
  "unknown_calls": z.int().min(0),
  "unpriced_calls": z.int().min(0),
});
