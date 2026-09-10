// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {FileListEntry} from "../models/FileListEntry.js";
import {FileEntryTypeSchema} from "./FileEntryType.js";

export const FileListEntrySchema: z.ZodType<FileListEntry> = z.object({
  "children": z.array(z.lazy(() => FileListEntrySchema)).nullable().exactOptional(),
  "is_text": z.boolean().nullable().exactOptional(),
  "label": z.string().nullable().exactOptional(),
  "mime": z.string().nullable().exactOptional(),
  "mtime": z.int().min(0),
  "name": z.string(),
  "path": z.string(),
  "size": z.int().min(0),
  "type": z.lazy(() => FileEntryTypeSchema),
});
