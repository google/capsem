// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ArchivedEventBody} from "../models/ArchivedEventBody.js";
import {BodyEncodingSchema} from "./BodyEncoding.js";

export const ArchivedEventBodySchema: z.ZodType<ArchivedEventBody> = z.object({
  "body_hash": z.string(),
  "content": z.string(),
  "content_type": z.string().nullable().exactOptional(),
  "direction": z.string(),
  "encoding": z.lazy(() => BodyEncodingSchema),
  "event_id": z.string(),
  "original_bytes": z.int().min(0),
  "source_table": z.string(),
  "stored_bytes": z.int().min(0),
  "truncated": z.boolean(),
  "truncated_for_transport": z.boolean(),
});
