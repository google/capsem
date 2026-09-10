// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SnapshotsList} from "../models/SnapshotsList.js";
import {SnapshotInfoSchema} from "./SnapshotInfo.js";

export const SnapshotsListSchema: z.ZodType<SnapshotsList> = z.object({
  "snapshots": z.array(z.lazy(() => SnapshotInfoSchema)),
  "total": z.int().min(0),
});
