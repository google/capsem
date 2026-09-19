// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {FileActionCount} from "../models/FileActionCount.js";
import {FileEventActionSchema} from "./FileEventAction.js";

export const FileActionCountSchema: z.ZodType<FileActionCount> = z.object({
  "action": z.lazy(() => FileEventActionSchema),
  "count": z.int().min(0),
});
