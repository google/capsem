// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmReadinessResponse} from "../models/VmReadinessResponse.js";
import {VmLifecycleSelectionSchema} from "./VmLifecycleSelection.js";

export const VmReadinessResponseSchema: z.ZodType<VmReadinessResponse> = z.strictObject({
  "selection": z.lazy(() => VmLifecycleSelectionSchema),
  "startup_timeout_seconds": z.int().min(0),
});
