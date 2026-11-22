// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentImageUpdateAction } from "./EnvironmentImageUpdateAction.js";

export interface EnvironmentImageUpdateRequest {
  "action": EnvironmentImageUpdateAction;
  "base_revision_id": string;
  "expected_digest": string;
  "reason": string;
  "request_id": string;
}
