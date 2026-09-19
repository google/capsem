// Generated from Capsem OpenAPI. Do not edit.

import type { TimelineLayer } from "./TimelineLayer.js";
import type { TimelineReference } from "./TimelineReference.js";
import type { TimelineStatus } from "./TimelineStatus.js";

export interface TimelineEvent {
  "duration_ms"?: number | null;
  "layer": TimelineLayer;
  "ref": TimelineReference;
  "status"?: null | TimelineStatus;
  "summary": string;
  "timestamp": string;
  "trace_id"?: string | null;
}
