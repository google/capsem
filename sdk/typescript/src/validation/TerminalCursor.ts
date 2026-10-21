// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {TerminalCursor} from "../models/TerminalCursor.js";

export const TerminalCursorSchema: z.ZodType<TerminalCursor> = z.object({
  "col": z.int().min(0),
  "hidden": z.boolean(),
  "row": z.int().min(0),
});
