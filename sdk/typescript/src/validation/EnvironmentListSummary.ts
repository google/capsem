// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentListSummary} from "../models/EnvironmentListSummary.js";
import {EnvironmentImageSchema} from "./EnvironmentImage.js";

export const EnvironmentListSummarySchema: z.ZodType<EnvironmentListSummary> = z.strictObject({
  "description": z.string(),
  "image": z.union([z.null(), z.lazy(() => EnvironmentImageSchema)]).exactOptional(),
  "name": z.string(),
  "timestamp_unix_ms": z.int(),
});
