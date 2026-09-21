// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {InteractionBody} from "../models/InteractionBody.js";
import {BodyDirectionSchema} from "./BodyDirection.js";
import {CapturedPayloadSchema} from "./CapturedPayload.js";

export const InteractionBodySchema: z.ZodType<InteractionBody> = z.object({
  "body_hash": z.string(),
  "content_type": z.string().nullable().exactOptional(),
  "direction": z.lazy(() => BodyDirectionSchema),
  "event_id": z.string(),
  "original_bytes": z.int().min(0),
  "payload": z.lazy(() => CapturedPayloadSchema),
  "stored_bytes": z.int().min(0),
});
