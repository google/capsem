// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentEffectiveAccessRule } from "./EnvironmentEffectiveAccessRule.js";
import type { EnvironmentEffectiveDetectionRule } from "./EnvironmentEffectiveDetectionRule.js";
import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";

export interface EnvironmentRuleListResponse {
  "access": Array<EnvironmentEffectiveAccessRule>;
  "detection": Array<EnvironmentEffectiveDetectionRule>;
  "revision": EnvironmentRevisionReference;
}
