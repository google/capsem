// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmFilesInfo} from "../models/VmFilesInfo.js";
import {FileActionCountSchema} from "./FileActionCount.js";

export const VmFilesInfoSchema: z.ZodType<VmFilesInfo> = z.object({
  "actions": z.array(z.lazy(() => FileActionCountSchema)),
  "total_events": z.int().min(0),
});
