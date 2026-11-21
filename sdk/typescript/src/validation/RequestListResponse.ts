// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RequestListResponse} from "../models/RequestListResponse.js";
import {RequestSummarySchema} from "./RequestSummary.js";

export const RequestListResponseSchema: z.ZodType<RequestListResponse> = z.strictObject({
  "next_cursor": z.int().nullable().exactOptional(),
  "requests": z.array(z.lazy(() => RequestSummarySchema)),
});
