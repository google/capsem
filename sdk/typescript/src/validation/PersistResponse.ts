// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PersistResponse} from "../models/PersistResponse.js";

export const PersistResponseSchema: z.ZodType<PersistResponse> = z.object({
  "name": z.string(),
  "success": z.boolean(),
});
