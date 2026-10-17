// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ErrorResponse} from "../models/ErrorResponse.js";
import {ErrorCodeSchema} from "./ErrorCode.js";

export const ErrorResponseSchema: z.ZodType<ErrorResponse> = z.object({
  "code": z.union([z.null(), z.lazy(() => ErrorCodeSchema)]).exactOptional(),
  "error": z.string(),
  "timeout_secs": z.int().min(0).nullable().exactOptional(),
  "vm_id": z.string().nullable().exactOptional(),
});
