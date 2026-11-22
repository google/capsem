// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentImageUpdateResponse} from "../models/EnvironmentImageUpdateResponse.js";
import {EnvironmentImageSchema} from "./EnvironmentImage.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";

export const EnvironmentImageUpdateResponseSchema: z.ZodType<EnvironmentImageUpdateResponse> = z.strictObject({
  "available": z.union([z.null(), z.lazy(() => EnvironmentImageSchema)]).exactOptional(),
  "current": z.union([z.null(), z.lazy(() => EnvironmentImageSchema)]).exactOptional(),
  "environment": z.lazy(() => EnvironmentRevisionReferenceSchema),
});
