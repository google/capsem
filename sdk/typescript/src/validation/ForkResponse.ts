// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ForkResponse} from "../models/ForkResponse.js";

export const ForkResponseSchema: z.ZodType<ForkResponse> = z.object({
  "id": z.string(),
  "name": z.string(),
  "size_bytes": z.int().min(0),
});
