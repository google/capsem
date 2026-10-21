// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {TerminalNotification} from "../models/TerminalNotification.js";

export const TerminalNotificationSchema: z.ZodType<TerminalNotification> = z.object({
  "body": z.string().max(1024),
  "id": z.string().max(1024),
  "title": z.string().max(1024),
});
