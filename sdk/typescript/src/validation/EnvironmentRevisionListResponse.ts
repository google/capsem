// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentRevisionListResponse} from "../models/EnvironmentRevisionListResponse.js";
import {EnvironmentRevisionSummarySchema} from "./EnvironmentRevisionSummary.js";

export const EnvironmentRevisionListResponseSchema: z.ZodType<EnvironmentRevisionListResponse> = z.strictObject({
  "next_cursor": z.int().nullable().exactOptional(),
  "revisions": z.array(z.lazy(() => EnvironmentRevisionSummarySchema)),
});
