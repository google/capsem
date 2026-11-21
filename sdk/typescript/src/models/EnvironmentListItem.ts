// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentListSummary } from "./EnvironmentListSummary.js";
import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";

export interface EnvironmentListItem {
  "archived": boolean;
  "environment": EnvironmentRevisionReference;
  "instance_count"?: number | null;
  "summary"?: null | EnvironmentListSummary;
}
