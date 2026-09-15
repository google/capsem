// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RunRequest} from "../models/RunRequest.js";

export const RunRequestSchema: z.ZodType<RunRequest> = z.object({
  "command": z.string(),
  "cpus": z.int().min(0).nullable().exactOptional(),
  "env": z.record(z.string(), z.string()).nullable().exactOptional(),
  "profile_id": z.string(),
  "ram_mb": z.int().min(0).nullable().exactOptional(),
  "timeout_secs": z.int().min(0).nullable().exactOptional(),
});
