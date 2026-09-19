// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {UpdateStatusResponse} from "../models/UpdateStatusResponse.js";
import {SupplyChainEvidenceSchema} from "./SupplyChainEvidence.js";
import {UpdateTrackStatusSchema} from "./UpdateTrackStatus.js";
import {ValidationStatusSchema} from "./ValidationStatus.js";

export const UpdateStatusResponseSchema: z.ZodType<UpdateStatusResponse> = z.object({
  "assets": z.lazy(() => UpdateTrackStatusSchema),
  "binary": z.lazy(() => UpdateTrackStatusSchema),
  "channel_hash": z.string().nullable().exactOptional(),
  "channel_url": z.string().nullable().exactOptional(),
  "checked_at": z.int().min(0).nullable().exactOptional(),
  "images": z.lazy(() => UpdateTrackStatusSchema),
  "last_error": z.string().nullable().exactOptional(),
  "profiles": z.lazy(() => UpdateTrackStatusSchema),
  "stale": z.boolean(),
  "supply_chain": z.lazy(() => SupplyChainEvidenceSchema),
  "validation_error": z.string().nullable().exactOptional(),
  "validation_status": z.union([z.null(), z.lazy(() => ValidationStatusSchema)]).exactOptional(),
});
