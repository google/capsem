// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {TerminalState} from "../models/TerminalState.js";
import {TerminalCursorSchema} from "./TerminalCursor.js";
import {TerminalEventSchema} from "./TerminalEvent.js";
import {TerminalModesSchema} from "./TerminalModes.js";
import {TerminalNotificationSchema} from "./TerminalNotification.js";

export const TerminalStateSchema: z.ZodType<TerminalState> = z.object({
  "bell_count": z.int().min(0),
  "cols": z.int().min(1).max(512),
  "cursor": z.lazy(() => TerminalCursorSchema),
  "events": z.array(z.lazy(() => TerminalEventSchema)).max(32),
  "icon_name": z.string().max(1024),
  "last_notification": z.union([z.null(), z.lazy(() => TerminalNotificationSchema)]).exactOptional(),
  "modes": z.lazy(() => TerminalModesSchema),
  "reset": z.boolean(),
  "revision": z.int().min(0),
  "rows": z.int().min(1).max(256),
  "title": z.string().max(1024),
});
