// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SnapshotInfo} from "../models/SnapshotInfo.js";
import {SnapshotOriginSchema} from "./SnapshotOrigin.js";

export const SnapshotInfoSchema: z.ZodType<SnapshotInfo> = z.object({
  "checkpoint": z.string(),
  "hash": z.string().nullable().exactOptional(),
  "name": z.string().nullable().exactOptional(),
  "origin": z.lazy(() => SnapshotOriginSchema),
  "slot": z.int().min(0),
  "timestamp": z.string(),
});
