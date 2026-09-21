// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ExposureListResponse} from "../models/ExposureListResponse.js";
import {ExposureInfoSchema} from "./ExposureInfo.js";

export const ExposureListResponseSchema: z.ZodType<ExposureListResponse> = z.object({
  "exposures": z.array(z.lazy(() => ExposureInfoSchema)),
  "owner_generation": z.string(),
});
