// Generated from Capsem OpenAPI. Do not edit.

import type { BodyDirection } from "./BodyDirection.js";

export interface EventBody {
  "body": string;
  "body_hash": string;
  "content_type"?: string | null;
  "direction": BodyDirection;
  "event_id": string;
  "original_bytes": number;
  "stored_bytes": number;
  "truncated": boolean;
}
