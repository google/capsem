// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PanicEvent} from "../models/PanicEvent.js";

export const PanicEventSchema: z.ZodType<PanicEvent> = z.object({
  "binary": z.string(),
  "frames": z.array(z.string()),
  "location": z.string().nullable().exactOptional(),
  "message": z.string(),
  "thread": z.string().nullable().exactOptional(),
  "ts": z.string(),
});
