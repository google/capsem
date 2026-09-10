// Generated from Capsem OpenAPI. Do not edit.

import type { InteractionContent } from "./InteractionContent.js";

export interface Interaction {
  "content": InteractionContent;
  "event_id": string;
  "item_index"?: number | null;
  "model_call_id"?: number | null;
  "model_event_id"?: string | null;
  "timestamp": string;
  "trace_id"?: string | null;
  "turn_id"?: string | null;
}
