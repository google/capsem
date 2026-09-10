// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {FileChange} from "../models/FileChange.js";
import {FileChangeKindSchema} from "./FileChangeKind.js";

export const FileChangeSchema: z.ZodType<FileChange> = z.object({
  "is_symlink": z.boolean(),
  "kind": z.lazy(() => FileChangeKindSchema),
  "path": z.string(),
  "size": z.int().min(0).nullable().exactOptional(),
});
