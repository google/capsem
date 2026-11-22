// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentSandboxTarget } from "./EnvironmentSandboxTarget.js";
import type { RequestApplicationComposition } from "./RequestApplicationComposition.js";
import type { RequestApplicationState } from "./RequestApplicationState.js";

export interface RequestTargetApplication {
  "composition"?: null | RequestApplicationComposition;
  "failure_reason"?: string | null;
  "state": RequestApplicationState;
  "target": EnvironmentSandboxTarget;
  "timestamp_unix_ms"?: number | null;
}
