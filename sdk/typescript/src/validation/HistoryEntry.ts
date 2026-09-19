// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {HistoryEntry} from "../models/HistoryEntry.js";
import {HistoryDetailsSchema} from "./HistoryDetails.js";
import {HistoryLayerSchema} from "./HistoryLayer.js";

export const HistoryEntrySchema: z.ZodType<HistoryEntry> = z.object({
  "command": z.string(),
  "details": z.lazy(() => HistoryDetailsSchema),
  "duration_ms": z.int().min(0).nullable().exactOptional(),
  "exit_code": z.int().nullable().exactOptional(),
  "layer": z.lazy(() => HistoryLayerSchema),
  "stderr_preview": z.string().nullable().exactOptional(),
  "stdout_preview": z.string().nullable().exactOptional(),
  "timestamp": z.string(),
});
