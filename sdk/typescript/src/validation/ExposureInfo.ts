// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ExposureInfo} from "../models/ExposureInfo.js";
import {ExposureTargetSchema} from "./ExposureTarget.js";

export const ExposureInfoSchema: z.ZodType<ExposureInfo> = z.object({
  "guest_port": z.int().min(0),
  "host_port": z.int().min(0),
  "id": z.string(),
  "target": z.lazy(() => ExposureTargetSchema),
});
