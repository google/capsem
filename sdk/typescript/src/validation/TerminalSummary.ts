// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {TerminalSummary} from "../models/TerminalSummary.js";
import {TerminalNotificationSchema} from "./TerminalNotification.js";

export const TerminalSummarySchema: z.ZodType<TerminalSummary> = z.object({
  "bell_count": z.int().min(0),
  "last_notification": z.union([z.null(), z.lazy(() => TerminalNotificationSchema)]).exactOptional(),
  "revision": z.int().min(0),
  "title": z.string(),
});
