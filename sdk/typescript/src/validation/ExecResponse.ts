// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ExecResponse} from "../models/ExecResponse.js";

export const ExecResponseSchema: z.ZodType<ExecResponse> = z.object({
  "exit_code": z.int(),
  "stderr": z.string(),
  "stdout": z.string(),
  "truncated": z.boolean().exactOptional(),
});
