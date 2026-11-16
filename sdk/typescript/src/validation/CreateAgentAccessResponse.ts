// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CreateAgentAccessResponse} from "../models/CreateAgentAccessResponse.js";
import {AgentAccessSummarySchema} from "./AgentAccessSummary.js";

export const CreateAgentAccessResponseSchema: z.ZodType<CreateAgentAccessResponse> = z.strictObject({
  "access": z.lazy(() => AgentAccessSummarySchema),
});
