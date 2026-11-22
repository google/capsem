// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentApplyTarget } from "./EnvironmentApplyTarget.js";

export interface EnvironmentApplyPreviewRequest {
  "base_revision_id": string;
  "revision_id": string;
  "targets": Array<EnvironmentApplyTarget>;
}
