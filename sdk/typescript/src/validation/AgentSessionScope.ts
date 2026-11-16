// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {AgentSessionScope} from "../models/AgentSessionScope.js";

export const AgentSessionScopeSchema: z.ZodType<AgentSessionScope> = z.object({
  "owner_generation": z.string(),
  "session_id": z.string(),
});
