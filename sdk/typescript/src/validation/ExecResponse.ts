// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ExecResponse} from "../models/ExecResponse.js";
import {ExecOutputSchema} from "./ExecOutput.js";

export const ExecResponseSchema: z.ZodType<ExecResponse> = z.object({
  "exit_code": z.int(),
  "stderr": z.lazy(() => ExecOutputSchema),
  "stdout": z.lazy(() => ExecOutputSchema),
  "truncated": z.boolean().exactOptional(),
});
