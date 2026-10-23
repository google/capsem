// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SecurityAskRequest} from "../models/SecurityAskRequest.js";
import {SecurityAskStatusSchema} from "./SecurityAskStatus.js";

export const SecurityAskRequestSchema: z.ZodType<SecurityAskRequest> = z.object({
  "ask_id": z.string(),
  "created_at_unix_ms": z.int(),
  "event_id": z.string(),
  "event_type": z.string(),
  "expires_at_unix_ms": z.int(),
  "resolution_reason": z.string().nullable().exactOptional(),
  "resolved_at_unix_ms": z.int().nullable().exactOptional(),
  "rule_id": z.string(),
  "rule_name": z.string(),
  "status": z.lazy(() => SecurityAskStatusSchema),
  "summary": z.string(),
});
