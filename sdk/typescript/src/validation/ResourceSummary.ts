// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ResourceSummary} from "../models/ResourceSummary.js";

export const ResourceSummarySchema: z.ZodType<ResourceSummary> = z.object({
  "running_count": z.int().min(0),
  "stopped_count": z.int().min(0),
  "suspended_count": z.int().min(0),
  "total_cpus": z.int().min(0),
  "total_ram_mb": z.int().min(0),
});
