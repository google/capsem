// Generated from Capsem OpenAPI. Do not edit.

import type { FileEventAction } from "./FileEventAction.js";

export interface FileEvent {
  "action": FileEventAction;
  "credential_ref"?: string | null;
  "event_id": string;
  "path": string;
  "size"?: number | null;
  "timestamp": string;
  "trace_id"?: string | null;
}
