// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {StreamControlFocus} from "../models/StreamControlFocus.js";

export const StreamControlFocusSchema: z.ZodType<StreamControlFocus> = z.object({
  "cols": z.int().min(1).max(512),
  "rows": z.int().min(1).max(256),
  "type": z.literal("focus"),
});
