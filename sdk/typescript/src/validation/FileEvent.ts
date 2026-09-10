// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {FileEvent} from "../models/FileEvent.js";
import {FileEventActionSchema} from "./FileEventAction.js";

export const FileEventSchema: z.ZodType<FileEvent> = z.object({
  "action": z.lazy(() => FileEventActionSchema),
  "credential_ref": z.string().nullable().exactOptional(),
  "event_id": z.string(),
  "path": z.string(),
  "size": z.int().min(0).nullable().exactOptional(),
  "timestamp": z.string(),
  "trace_id": z.string().nullable().exactOptional(),
});
