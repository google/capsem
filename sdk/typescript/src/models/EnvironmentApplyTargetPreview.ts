// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentApplyTarget } from "./EnvironmentApplyTarget.js";
import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";

export interface EnvironmentApplyTargetPreview {
  "boot": EnvironmentRevisionReference;
  "live_changes": Array<string>;
  "live_policy": EnvironmentRevisionReference;
  "next_start_changes": Array<string>;
  "target": EnvironmentApplyTarget;
}
