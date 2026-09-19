// Generated from Capsem OpenAPI. Do not edit.

import type { BodyDirection } from "./BodyDirection.js";
import type { CapturedPayload } from "./CapturedPayload.js";

export interface InteractionBody {
  "body_hash": string;
  "content_type"?: string | null;
  "direction": BodyDirection;
  "event_id": string;
  "original_bytes": number;
  "payload": CapturedPayload;
  "stored_bytes": number;
}
