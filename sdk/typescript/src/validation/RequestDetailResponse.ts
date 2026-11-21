// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RequestDetailResponse} from "../models/RequestDetailResponse.js";
import {EnvironmentReviewContentSchema} from "./EnvironmentReviewContent.js";
import {EnvironmentSandboxTargetSchema} from "./EnvironmentSandboxTarget.js";
import {RequestDecisionDetailsSchema} from "./RequestDecisionDetails.js";
import {RequestSummarySchema} from "./RequestSummary.js";

export const RequestDetailResponseSchema: z.ZodType<RequestDetailResponse> = z.strictObject({
  "base": z.union([z.null(), z.lazy(() => EnvironmentReviewContentSchema)]).exactOptional(),
  "disposition": z.union([z.null(), z.lazy(() => RequestDecisionDetailsSchema)]).exactOptional(),
  "observed_revision": z.string(),
  "proposed": z.lazy(() => EnvironmentReviewContentSchema),
  "summary": z.lazy(() => RequestSummarySchema),
  "targets": z.array(z.lazy(() => EnvironmentSandboxTargetSchema)),
});
