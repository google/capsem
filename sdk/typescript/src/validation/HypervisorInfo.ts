// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {HypervisorInfo} from "../models/HypervisorInfo.js";
import {AssetStatusSchema} from "./AssetStatus.js";
import {ResourceSummarySchema} from "./ResourceSummary.js";
import {ServiceAvailabilitySchema} from "./ServiceAvailability.js";
import {UpdateStatusResponseSchema} from "./UpdateStatusResponse.js";
import {VmSummarySchema} from "./VmSummary.js";

export const HypervisorInfoSchema: z.ZodType<HypervisorInfo> = z.object({
  "assets": z.union([z.null(), z.lazy(() => AssetStatusSchema)]).exactOptional(),
  "gateway_version": z.string(),
  "resource_summary": z.union([z.null(), z.lazy(() => ResourceSummarySchema)]).exactOptional(),
  "service": z.lazy(() => ServiceAvailabilitySchema),
  "updates": z.union([z.null(), z.lazy(() => UpdateStatusResponseSchema)]).exactOptional(),
  "vm_count": z.int().min(0),
  "vms": z.array(z.lazy(() => VmSummarySchema)),
});
