// Generated from Capsem OpenAPI. Do not edit.

import type { TerminalEventKind } from "./TerminalEventKind.js";
import type { TerminalNotification } from "./TerminalNotification.js";

export interface TerminalEvent {
  "kind": TerminalEventKind;
  "notification"?: null | TerminalNotification;
  "visual": boolean;
}
