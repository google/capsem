// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";

export interface EnvironmentPromotionRequest {
  "base_revision_id": string;
  "reason": string;
  "request_id": string;
  "source": EnvironmentRevisionReference;
}
