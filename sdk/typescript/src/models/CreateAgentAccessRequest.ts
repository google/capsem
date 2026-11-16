// Generated from Capsem OpenAPI. Do not edit.

import type { AgentAccessScope } from "./AgentAccessScope.js";

export interface CreateAgentAccessRequest {
  "lease_seconds": number;
  "ownership_token": string;
  "request_id": string;
  "scope": AgentAccessScope;
}
