// Generated from Capsem OpenAPI. Do not edit.

import type { CapabilitySummary } from "./CapabilitySummary.js";
import type { EnrollmentKind } from "./EnrollmentKind.js";

export interface ScopeProfileSummary {
  "application_label": string;
  "capabilities": Array<CapabilitySummary>;
  "enabled": boolean;
  "enrollment_kind": EnrollmentKind;
  "id": string;
  "label": string;
  "requested_scopes": Array<string>;
}
