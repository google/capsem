// Generated from Capsem OpenAPI. Do not edit.

import type { AgentOperation } from "./AgentOperation.js";
import type { AgentSessionScope } from "./AgentSessionScope.js";

export interface AgentAccessScope {
  "environment_ids": Array<string>;
  "operations": Array<AgentOperation>;
  "sessions": Array<AgentSessionScope>;
}
