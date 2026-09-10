// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ExecRequest} from "../models/ExecRequest.js";

export const ExecRequestSchema: z.ZodType<ExecRequest> = z.object({
  "command": z.string(),
  "timeout_secs": z.int().min(0).nullable().exactOptional(),
});
