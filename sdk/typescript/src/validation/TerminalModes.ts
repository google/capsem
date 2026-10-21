// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {TerminalModes} from "../models/TerminalModes.js";

export const TerminalModesSchema: z.ZodType<TerminalModes> = z.object({
  "alternate_screen": z.boolean(),
  "application_cursor": z.boolean(),
  "application_keypad": z.boolean(),
  "bracketed_paste": z.boolean(),
  "mouse_encoding": z.string().max(1024),
  "mouse_protocol": z.string().max(1024),
});
