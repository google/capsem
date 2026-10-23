// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SecurityAskListResponse} from "../models/SecurityAskListResponse.js";
import {SecurityAskRequestSchema} from "./SecurityAskRequest.js";

export const SecurityAskListResponseSchema: z.ZodType<SecurityAskListResponse> = z.object({
  "generation": z.string(),
  "pending_count": z.int().min(0),
  "requests": z.array(z.lazy(() => SecurityAskRequestSchema)),
  "vm_id": z.string(),
});
