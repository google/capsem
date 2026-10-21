// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {StreamControlCloseStdin} from "../models/StreamControlCloseStdin.js";

export const StreamControlCloseStdinSchema: z.ZodType<StreamControlCloseStdin> = z.object({
  "type": z.literal("close_stdin"),
});
