// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ErrorResponse} from "../models/ErrorResponse.js";

export const ErrorResponseSchema: z.ZodType<ErrorResponse> = z.object({
  "code": z.string().nullable().exactOptional(),
  "error": z.string(),
});
