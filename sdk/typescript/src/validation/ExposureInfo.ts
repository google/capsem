// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ExposureInfo} from "../models/ExposureInfo.js";
import {ExposureAccessSchema} from "./ExposureAccess.js";
import {ExposureTargetSchema} from "./ExposureTarget.js";

export const ExposureInfoSchema: z.ZodType<ExposureInfo> = z.object({
  "access": z.lazy(() => ExposureAccessSchema),
  "guest_port": z.int().min(0),
  "host_port": z.int().min(0).nullable().exactOptional(),
  "id": z.string(),
  "target": z.lazy(() => ExposureTargetSchema),
});
