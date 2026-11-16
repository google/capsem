// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {AgentAccessScope} from "../models/AgentAccessScope.js";
import {AgentOperationSchema} from "./AgentOperation.js";
import {AgentSessionScopeSchema} from "./AgentSessionScope.js";

export const AgentAccessScopeSchema: z.ZodType<AgentAccessScope> = z.object({
  "environment_ids": z.array(z.string()),
  "operations": z.array(z.lazy(() => AgentOperationSchema)),
  "sessions": z.array(z.lazy(() => AgentSessionScopeSchema)),
});
