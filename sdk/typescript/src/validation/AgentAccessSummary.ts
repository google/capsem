// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {AgentAccessSummary} from "../models/AgentAccessSummary.js";
import {AgentAccessScopeSchema} from "./AgentAccessScope.js";

export const AgentAccessSummarySchema: z.ZodType<AgentAccessSummary> = z.strictObject({
  "expires_at": z.string(),
  "id": z.string(),
  "scope": z.lazy(() => AgentAccessScopeSchema),
});
