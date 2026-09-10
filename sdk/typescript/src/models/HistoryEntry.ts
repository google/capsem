// Generated from Capsem OpenAPI. Do not edit.

import type { HistoryDetails } from "./HistoryDetails.js";
import type { HistoryLayer } from "./HistoryLayer.js";

export interface HistoryEntry {
  "command": string;
  "details": HistoryDetails;
  "duration_ms"?: number | null;
  "exit_code"?: number | null;
  "layer": HistoryLayer;
  "stderr_preview"?: string | null;
  "stdout_preview"?: string | null;
  "timestamp": string;
}
