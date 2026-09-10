// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {UpdateTrackStatus} from "../models/UpdateTrackStatus.js";
import {UpdateCompatibilityStateSchema} from "./UpdateCompatibilityState.js";
import {UpdateTrackStateSchema} from "./UpdateTrackState.js";

export const UpdateTrackStatusSchema: z.ZodType<UpdateTrackStatus> = z.object({
  "blocked_reason": z.string().nullable().exactOptional(),
  "compatibility": z.lazy(() => UpdateCompatibilityStateSchema),
  "current": z.string().nullable().exactOptional(),
  "latest": z.string().nullable().exactOptional(),
  "state": z.lazy(() => UpdateTrackStateSchema),
  "update_available": z.boolean(),
});
