// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {NetworkLogEvent} from "../models/NetworkLogEvent.js";

export const NetworkLogEventSchema: z.ZodType<NetworkLogEvent> = z.object({
  "connection_id": z.string().nullable().exactOptional(),
  "event": z.json(),
  "event_id": z.string(),
  "event_type": z.string(),
  "sequence": z.int(),
  "timestamp_unix_ms": z.int(),
});
