// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {TerminalNotification} from "../models/TerminalNotification.js";

export const TerminalNotificationSchema: z.ZodType<TerminalNotification> = z.object({
  "body": z.string(),
  "id": z.string(),
  "title": z.string(),
});
