// Generated from Capsem OpenAPI. Do not edit.

import type { DetectionLevel } from "./DetectionLevel.js";
import type { EnvironmentRuleSource } from "./EnvironmentRuleSource.js";
import type { SecurityRuleAction } from "./SecurityRuleAction.js";

export interface EnvironmentEffectiveAccessRule {
  "action": SecurityRuleAction;
  "condition": string;
  "detection_level": null | DetectionLevel;
  "editable_key": string | null;
  "enabled": boolean;
  "id": string;
  "locked": boolean;
  "name": string;
  "priority": number;
  "source": EnvironmentRuleSource;
}
