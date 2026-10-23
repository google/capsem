// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ModelUsage} from "../models/ModelUsage.js";
import {CostCoverageSchema} from "./CostCoverage.js";

export const ModelUsageSchema: z.ZodType<ModelUsage> = z.object({
  "call_count": z.int().min(0),
  "cost_coverage": z.union([z.null(), z.lazy(() => CostCoverageSchema)]).exactOptional(),
  "duration_ms": z.int().min(0),
  "estimated_cost_usd": z.number(),
  "input_tokens": z.int().min(0),
  "model": z.string(),
  "output_tokens": z.int().min(0),
  "provider": z.string(),
});
