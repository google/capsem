// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ForkRequest} from "../models/ForkRequest.js";
import {VmLifecycleSelectionSchema} from "./VmLifecycleSelection.js";

export const ForkRequestSchema: z.ZodType<ForkRequest> = z.strictObject({
  "description": z.string().nullable().exactOptional(),
  "expected": z.union([z.null(), z.lazy(() => VmLifecycleSelectionSchema)]).exactOptional(),
  "labels": z.record(z.string(), z.string()).nullable().exactOptional(),
  "name": z.string(),
});
