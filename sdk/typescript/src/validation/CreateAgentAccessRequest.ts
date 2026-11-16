// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CreateAgentAccessRequest} from "../models/CreateAgentAccessRequest.js";
import {AgentAccessScopeSchema} from "./AgentAccessScope.js";

export const CreateAgentAccessRequestSchema: z.ZodType<CreateAgentAccessRequest> = z.strictObject({
  "lease_seconds": z.int().min(0),
  "ownership_token": z.string().length(64).regex(new RegExp("^[0-9a-f]{64}$")),
  "request_id": z.uuid(),
  "scope": z.lazy(() => AgentAccessScopeSchema),
});
