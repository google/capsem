// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {StreamControlResize} from "../models/StreamControlResize.js";

export const StreamControlResizeSchema: z.ZodType<StreamControlResize> = z.object({
  "cols": z.int().min(1).max(512),
  "rows": z.int().min(1).max(256),
  "type": z.literal("resize"),
});
