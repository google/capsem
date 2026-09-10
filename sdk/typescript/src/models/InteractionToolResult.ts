// Generated from Capsem OpenAPI. Do not edit.

import type { CapturedPayload } from "./CapturedPayload.js";
import type { InteractionToolResultKind } from "./InteractionToolResultKind.js";

export interface InteractionToolResult {
  "call_id": string;
  "error_message"?: string | null;
  "is_error"?: boolean | null;
  "kind": InteractionToolResultKind;
  "payload"?: null | CapturedPayload;
  "response"?: null | CapturedPayload;
}
