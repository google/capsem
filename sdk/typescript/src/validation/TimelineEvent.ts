// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {TimelineEvent} from "../models/TimelineEvent.js";
import {TimelineLayerSchema} from "./TimelineLayer.js";
import {TimelineReferenceSchema} from "./TimelineReference.js";
import {TimelineStatusSchema} from "./TimelineStatus.js";

export const TimelineEventSchema: z.ZodType<TimelineEvent> = z.object({
  "duration_ms": z.int().min(0).nullable().exactOptional(),
  "layer": z.lazy(() => TimelineLayerSchema),
  "ref": z.lazy(() => TimelineReferenceSchema),
  "status": z.union([z.null(), z.lazy(() => TimelineStatusSchema)]).exactOptional(),
  "summary": z.string(),
  "timestamp": z.string(),
  "trace_id": z.string().nullable().exactOptional(),
});
