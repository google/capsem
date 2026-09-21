// Generated from Capsem OpenAPI. Do not edit.

import type { UpdateActionStatus } from "./UpdateActionStatus.js";
import type { UpdateCommandPlan } from "./UpdateCommandPlan.js";

export interface UpdateActionResponse {
  "command": UpdateCommandPlan;
  "exit_code"?: number | null;
  "status": UpdateActionStatus;
  "stderr"?: string | null;
  "stdout"?: string | null;
}
