// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {TerminalEvent} from "../models/TerminalEvent.js";
import {TerminalEventKindSchema} from "./TerminalEventKind.js";
import {TerminalNotificationSchema} from "./TerminalNotification.js";

export const TerminalEventSchema: z.ZodType<TerminalEvent> = z.object({
  "kind": z.lazy(() => TerminalEventKindSchema),
  "notification": z.union([z.null(), z.lazy(() => TerminalNotificationSchema)]).exactOptional(),
  "visual": z.boolean(),
});
