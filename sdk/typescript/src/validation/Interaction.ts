// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {Interaction} from "../models/Interaction.js";
import {InteractionContentSchema} from "./InteractionContent.js";

export const InteractionSchema: z.ZodType<Interaction> = z.object({
  "content": z.lazy(() => InteractionContentSchema),
  "event_id": z.string(),
  "item_index": z.int().min(0).nullable().exactOptional(),
  "model_call_id": z.int().nullable().exactOptional(),
  "model_event_id": z.string().nullable().exactOptional(),
  "timestamp": z.string(),
  "trace_id": z.string().nullable().exactOptional(),
  "turn_id": z.string().nullable().exactOptional(),
});
