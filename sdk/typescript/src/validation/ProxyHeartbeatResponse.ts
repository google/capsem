// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProxyHeartbeatResponse} from "../models/ProxyHeartbeatResponse.js";

export const ProxyHeartbeatResponseSchema: z.ZodType<ProxyHeartbeatResponse> = z.object({
  "lease_expires_unix_ms": z.int().min(0),
  "session_id": z.string(),
});
