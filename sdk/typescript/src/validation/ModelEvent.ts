// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ModelEvent} from "../models/ModelEvent.js";

export const ModelEventSchema: z.ZodType<ModelEvent> = z.object({
  "credential_ref": z.string().nullable().exactOptional(),
  "duration_ms": z.int().min(0).nullable().exactOptional(),
  "event_id": z.string(),
  "input_tokens": z.int().min(0).nullable().exactOptional(),
  "method": z.string(),
  "model": z.string().nullable().exactOptional(),
  "output_tokens": z.int().min(0).nullable().exactOptional(),
  "path": z.string(),
  "provider": z.string(),
  "response_bytes": z.int().min(0).nullable().exactOptional(),
  "status_code": z.int().min(0).nullable().exactOptional(),
  "stop_reason": z.string().nullable().exactOptional(),
  "timestamp": z.string(),
  "trace_id": z.string().nullable().exactOptional(),
});
