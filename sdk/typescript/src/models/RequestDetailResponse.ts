// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentReviewContent } from "./EnvironmentReviewContent.js";
import type { EnvironmentSandboxTarget } from "./EnvironmentSandboxTarget.js";
import type { RequestDecisionDetails } from "./RequestDecisionDetails.js";
import type { RequestSummary } from "./RequestSummary.js";

export interface RequestDetailResponse {
  "base"?: null | EnvironmentReviewContent;
  "disposition"?: null | RequestDecisionDetails;
  "observed_revision": string;
  "proposed": EnvironmentReviewContent;
  "summary": RequestSummary;
  "targets": Array<EnvironmentSandboxTarget>;
}
