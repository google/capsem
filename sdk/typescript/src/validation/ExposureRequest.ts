// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ExposureRequest} from "../models/ExposureRequest.js";
import {ExposureTargetSchema} from "./ExposureTarget.js";

export const ExposureRequestSchema: z.ZodType<ExposureRequest> = z.object({
  "guest_port": z.int().min(0),
  "host_port": z.int().min(0).exactOptional(),
  "target": z.lazy(() => ExposureTargetSchema).exactOptional(),
});
