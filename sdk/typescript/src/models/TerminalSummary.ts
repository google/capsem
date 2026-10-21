// Generated from Capsem OpenAPI. Do not edit.

import type { TerminalNotification } from "./TerminalNotification.js";

export interface TerminalSummary {
  "bell_count": number;
  "last_notification"?: null | TerminalNotification;
  "revision": number;
  "title": string;
}
