// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentReviewContent } from "./EnvironmentReviewContent.js";
import type { EnvironmentSandboxTarget } from "./EnvironmentSandboxTarget.js";
import type { RequestDecisionDetails } from "./RequestDecisionDetails.js";
import type { RequestSummary } from "./RequestSummary.js";
import type { RequestTargetApplication } from "./RequestTargetApplication.js";

export interface RequestDetailResponse {
  "applications"?: Array<RequestTargetApplication>;
  "base"?: null | EnvironmentReviewContent;
  "disposition"?: null | RequestDecisionDetails;
  "observed_revision": string;
  "proposed": EnvironmentReviewContent;
  "restart_required"?: boolean | null;
  "summary": RequestSummary;
  "targets": Array<EnvironmentSandboxTarget>;
}
