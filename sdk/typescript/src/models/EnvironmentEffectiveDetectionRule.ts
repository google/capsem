// Generated from Capsem OpenAPI. Do not edit.

import type { DetectionLevel } from "./DetectionLevel.js";
import type { EnvironmentRuleSource } from "./EnvironmentRuleSource.js";

export interface EnvironmentEffectiveDetectionRule {
  "condition": string;
  "editable_key": string | null;
  "enabled": boolean;
  "id": string;
  "level": DetectionLevel;
  "name": string;
  "source": EnvironmentRuleSource;
}
