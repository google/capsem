// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {UpdateActionResponse} from "../models/UpdateActionResponse.js";
import {UpdateActionStatusSchema} from "./UpdateActionStatus.js";
import {UpdateCommandPlanSchema} from "./UpdateCommandPlan.js";

export const UpdateActionResponseSchema: z.ZodType<UpdateActionResponse> = z.object({
  "command": z.lazy(() => UpdateCommandPlanSchema),
  "exit_code": z.int().nullable().exactOptional(),
  "status": z.lazy(() => UpdateActionStatusSchema),
  "stderr": z.string().nullable().exactOptional(),
  "stdout": z.string().nullable().exactOptional(),
});
