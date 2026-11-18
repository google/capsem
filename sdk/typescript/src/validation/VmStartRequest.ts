// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmStartRequest} from "../models/VmStartRequest.js";
import {VmLifecycleSelectionSchema} from "./VmLifecycleSelection.js";

export const VmStartRequestSchema: z.ZodType<VmStartRequest> = z.strictObject({
  "expected": z.union([z.null(), z.lazy(() => VmLifecycleSelectionSchema)]).exactOptional(),
});
