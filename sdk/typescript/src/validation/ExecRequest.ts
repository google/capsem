// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ExecRequest} from "../models/ExecRequest.js";
import {ExecTargetSchema} from "./ExecTarget.js";

export const ExecRequestSchema: z.ZodType<ExecRequest> = z.object({
  "command": z.string(),
  "target": z.union([z.null(), z.lazy(() => ExecTargetSchema)]).exactOptional(),
  "timeout_secs": z.int().min(0).nullable().exactOptional(),
});
