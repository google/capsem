// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {FileListResponse} from "../models/FileListResponse.js";
import {FileListEntrySchema} from "./FileListEntry.js";

export const FileListResponseSchema: z.ZodType<FileListResponse> = z.object({
  "entries": z.array(z.lazy(() => FileListEntrySchema)),
});
