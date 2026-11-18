// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmLifecycleSelection} from "../models/VmLifecycleSelection.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";

export const VmLifecycleSelectionSchema: z.ZodType<VmLifecycleSelection> = z.strictObject({
  "environment": z.union([z.null(), z.lazy(() => EnvironmentRevisionReferenceSchema)]).exactOptional(),
  "generation": z.uuid().nullable().exactOptional(),
  "id": z.string(),
  "running": z.boolean(),
});
