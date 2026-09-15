// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SlowOpEvent} from "../models/SlowOpEvent.js";

export const SlowOpEventSchema: z.ZodType<SlowOpEvent> = z.object({
  "binary": z.string(),
  "duration_ms": z.int().min(0),
  "op": z.string(),
  "ts": z.string(),
});
