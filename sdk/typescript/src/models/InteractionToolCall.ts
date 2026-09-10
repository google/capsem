// Generated from Capsem OpenAPI. Do not edit.

import type { CapturedPayload } from "./CapturedPayload.js";
import type { InteractionToolCallKind } from "./InteractionToolCallKind.js";
import type { InteractionToolResult } from "./InteractionToolResult.js";
import type { ToolDecision } from "./ToolDecision.js";
import type { ToolOrigin } from "./ToolOrigin.js";

export interface InteractionToolCall {
  "arguments"?: null | CapturedPayload;
  "call_id": string;
  "decision": ToolDecision;
  "kind": InteractionToolCallKind;
  "origin": ToolOrigin;
  "request"?: null | CapturedPayload;
  "result"?: null | InteractionToolResult;
  "server_name"?: string | null;
  "tool_name": string;
}
