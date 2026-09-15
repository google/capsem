// Generated from Capsem OpenAPI. Do not edit.

import type { SnapshotInfo } from "./SnapshotInfo.js";

export interface SnapshotsStatus {
  "auto_count": number;
  "manual_available": number;
  "manual_count": number;
  "snapshots": Array<SnapshotInfo>;
  "total": number;
}
