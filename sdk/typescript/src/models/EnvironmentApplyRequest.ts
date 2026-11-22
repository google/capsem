// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentApplyTarget } from "./EnvironmentApplyTarget.js";

export interface EnvironmentApplyRequest {
  "base_revision_id": string;
  "reason": string;
  "request_id": string;
  "revision_id": string;
  "targets": Array<EnvironmentApplyTarget>;
}
