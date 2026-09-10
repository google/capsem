// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SnapshotsStatus} from "../models/SnapshotsStatus.js";
import {SnapshotInfoSchema} from "./SnapshotInfo.js";

export const SnapshotsStatusSchema: z.ZodType<SnapshotsStatus> = z.object({
  "auto_count": z.int().min(0),
  "manual_available": z.int().min(0),
  "manual_count": z.int().min(0),
  "snapshots": z.array(z.lazy(() => SnapshotInfoSchema)),
  "total": z.int().min(0),
});
