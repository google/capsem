// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentReadinessResponse} from "../models/EnvironmentReadinessResponse.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";

export const EnvironmentReadinessResponseSchema: z.ZodType<EnvironmentReadinessResponse> = z.strictObject({
  "environment": z.lazy(() => EnvironmentRevisionReferenceSchema),
  "startup_timeout_seconds": z.int().min(0),
});
