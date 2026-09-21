// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ForkRequest} from "../models/ForkRequest.js";

export const ForkRequestSchema: z.ZodType<ForkRequest> = z.object({
  "description": z.string().nullable().exactOptional(),
  "name": z.string(),
});
