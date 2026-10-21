// Generated from Capsem OpenAPI. Do not edit.

import type { TerminalCursor } from "./TerminalCursor.js";
import type { TerminalEvent } from "./TerminalEvent.js";
import type { TerminalModes } from "./TerminalModes.js";
import type { TerminalNotification } from "./TerminalNotification.js";

export interface TerminalState {
  "bell_count": number;
  "cols": number;
  "cursor": TerminalCursor;
  "events": Array<TerminalEvent>;
  "icon_name": string;
  "last_notification"?: null | TerminalNotification;
  "modes": TerminalModes;
  "reset": boolean;
  "revision": number;
  "rows": number;
  "title": string;
}
