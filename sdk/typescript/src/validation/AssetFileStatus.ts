// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {AssetFileStatus} from "../models/AssetFileStatus.js";
import {AssetFileStateSchema} from "./AssetFileState.js";

export const AssetFileStatusSchema: z.ZodType<AssetFileStatus> = z.object({
  "actual_size": z.int().min(0).nullable().exactOptional(),
  "expected_hash": z.string(),
  "expected_size": z.int().min(0).nullable().exactOptional(),
  "kind": z.string(),
  "name": z.string(),
  "path": z.string(),
  "status": z.lazy(() => AssetFileStateSchema),
});
