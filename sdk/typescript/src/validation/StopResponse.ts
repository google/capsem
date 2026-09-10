// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {StopResponse} from "../models/StopResponse.js";

export const StopResponseSchema: z.ZodType<StopResponse> = z.object({
  "persistent": z.boolean(),
  "success": z.boolean(),
});
