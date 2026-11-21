// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";
import type { EnvironmentSandboxTarget } from "./EnvironmentSandboxTarget.js";
import type { RequestApplicationState } from "./RequestApplicationState.js";
import type { RequestKind } from "./RequestKind.js";
import type { RequestState } from "./RequestState.js";

export interface RequestSummary {
  "actor": string;
  "application": RequestApplicationState;
  "base"?: null | EnvironmentRevisionReference;
  "decision": RequestState;
  "environment_id": string;
  "expires_at_unix_ms": number;
  "kind": RequestKind;
  "reason": string;
  "request_id": string;
  "requester"?: null | EnvironmentSandboxTarget;
  "result"?: null | EnvironmentRevisionReference;
  "source"?: null | EnvironmentRevisionReference;
  "timestamp_unix_ms": number;
}
