// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentCapabilityDecision } from "./EnvironmentCapabilityDecision.js";

export interface EnvironmentMcpPermissions {
  "default"?: EnvironmentCapabilityDecision;
  "enabled"?: boolean;
  "tools"?: Record<string, EnvironmentCapabilityDecision>;
}
