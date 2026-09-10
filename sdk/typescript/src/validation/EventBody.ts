// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EventBody} from "../models/EventBody.js";
import {BodyDirectionSchema} from "./BodyDirection.js";

export const EventBodySchema: z.ZodType<EventBody> = z.object({
  "body": z.string(),
  "body_hash": z.string(),
  "content_type": z.string().nullable().exactOptional(),
  "direction": z.lazy(() => BodyDirectionSchema),
  "event_id": z.string(),
  "original_bytes": z.int().min(0),
  "stored_bytes": z.int().min(0),
  "truncated": z.boolean(),
});
