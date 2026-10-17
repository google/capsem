// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ConnectionListResponse} from "../models/ConnectionListResponse.js";
import {ConnectionSummarySchema} from "./ConnectionSummary.js";
import {ScopeProfileSummarySchema} from "./ScopeProfileSummary.js";

export const ConnectionListResponseSchema: z.ZodType<ConnectionListResponse> = z.strictObject({
  "connections": z.array(z.lazy(() => ConnectionSummarySchema)),
  "next_cursor": z.string().nullable(),
  "scope_profiles": z.array(z.lazy(() => ScopeProfileSummarySchema)),
});
