// Generated from Capsem OpenAPI. Do not edit.

import type { JSONType } from "zod";

export interface NetworkLogEvent {
  "connection_id"?: string | null;
  "event": JSONType;
  "event_id": string;
  "event_type": string;
  "sequence": number;
  "timestamp_unix_ms": number;
}
