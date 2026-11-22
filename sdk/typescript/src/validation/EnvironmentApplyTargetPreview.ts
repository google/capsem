// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentApplyTargetPreview} from "../models/EnvironmentApplyTargetPreview.js";
import {EnvironmentApplyTargetSchema} from "./EnvironmentApplyTarget.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";

export const EnvironmentApplyTargetPreviewSchema: z.ZodType<EnvironmentApplyTargetPreview> = z.object({
  "boot": z.lazy(() => EnvironmentRevisionReferenceSchema),
  "live_changes": z.array(z.string()),
  "live_policy": z.lazy(() => EnvironmentRevisionReferenceSchema),
  "next_start_changes": z.array(z.string()),
  "target": z.lazy(() => EnvironmentApplyTargetSchema),
});
