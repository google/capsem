// Generated from Capsem OpenAPI. Do not edit.

import type { AssetFileState } from "./AssetFileState.js";

export interface AssetFileStatus {
  "actual_size"?: number | null;
  "expected_hash": string;
  "expected_size"?: number | null;
  "kind": string;
  "name": string;
  "path": string;
  "status": AssetFileState;
}
