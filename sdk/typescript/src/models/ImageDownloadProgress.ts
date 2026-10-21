// Generated from Capsem OpenAPI. Do not edit.

import type { ImageDownloadPhase } from "./ImageDownloadPhase.js";

export interface ImageDownloadProgress {
  "completed_bytes": number;
  "phase": ImageDownloadPhase;
  "total_bytes"?: number | null;
}
