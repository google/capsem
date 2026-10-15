// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SessionGrantListResponse} from "../models/SessionGrantListResponse.js";
import {SessionGrantSummarySchema} from "./SessionGrantSummary.js";

export const SessionGrantListResponseSchema: z.ZodType<SessionGrantListResponse> = z.strictObject({
  "denied_capability_ids": z.array(z.string()),
  "grants": z.array(z.lazy(() => SessionGrantSummarySchema)),
  "owner_generation": z.string(),
  "session_id": z.string(),
});
