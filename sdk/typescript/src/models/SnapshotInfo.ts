// Generated from Capsem OpenAPI. Do not edit.

import type { SnapshotOrigin } from "./SnapshotOrigin.js";

export interface SnapshotInfo {
  "checkpoint": string;
  "hash"?: string | null;
  "name"?: string | null;
  "origin": SnapshotOrigin;
  "slot": number;
  "timestamp": string;
}
