// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";
import type { RequestApplicationState } from "./RequestApplicationState.js";
import type { RequestState } from "./RequestState.js";

export interface EnvironmentMutationResponse {
  "application": RequestApplicationState;
  "application_error"?: string | null;
  "decision": RequestState;
  "environment_id": string;
  "request_id": string;
  "result"?: null | EnvironmentRevisionReference;
}
