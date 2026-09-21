// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {UpdateCommandPlan} from "../models/UpdateCommandPlan.js";

export const UpdateCommandPlanSchema: z.ZodType<UpdateCommandPlan> = z.object({
  "args": z.array(z.string()),
  "program": z.string(),
});
