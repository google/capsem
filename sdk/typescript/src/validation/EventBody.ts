// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EventBody} from "../models/EventBody.js";

export const EventBodySchema: z.ZodType<EventBody> = z.object({
  "body_hash": z.string(),
  "content_type": z.string().nullable().exactOptional(),
  "direction": z.string(),
  "event_id": z.string(),
  "original_bytes": z.int().min(0),
  "source_table": z.string(),
  "stored_bytes": z.int().min(0),
  "truncated": z.boolean(),
});
