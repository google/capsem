// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";

export interface EnvironmentRevisionSummary {
  "actor": string;
  "environment": EnvironmentRevisionReference;
  "fork_parent"?: null | EnvironmentRevisionReference;
  "previous_revision_id"?: string | null;
  "reason": string;
  "replayable": boolean;
  "request_id"?: string | null;
  "timestamp_unix_ms": number;
}
