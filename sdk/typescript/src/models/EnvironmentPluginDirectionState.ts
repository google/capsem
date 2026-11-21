// Generated from Capsem OpenAPI. Do not edit.

import type { DetectionLevel } from "./DetectionLevel.js";
import type { PluginDirection } from "./PluginDirection.js";
import type { SecurityPluginMode } from "./SecurityPluginMode.js";

export interface EnvironmentPluginDirectionState {
  "detection_level": null | DetectionLevel;
  "direction": PluginDirection;
  "mode": SecurityPluginMode;
}
