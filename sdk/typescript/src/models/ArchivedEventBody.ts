// Generated from Capsem OpenAPI. Do not edit.

import type { BodyEncoding } from "./BodyEncoding.js";

export interface ArchivedEventBody {
  "body_hash": string;
  "content": string;
  "content_type"?: string | null;
  "direction": string;
  "encoding": BodyEncoding;
  "event_id": string;
  "original_bytes": number;
  "source_table": string;
  "stored_bytes": number;
  "truncated": boolean;
  "truncated_for_transport": boolean;
}
