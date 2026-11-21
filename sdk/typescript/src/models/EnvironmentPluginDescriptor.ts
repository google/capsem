// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentPluginField } from "./EnvironmentPluginField.js";
import type { PluginControlGroup } from "./PluginControlGroup.js";
import type { PluginDetailCapability } from "./PluginDetailCapability.js";
import type { PluginDirection } from "./PluginDirection.js";
import type { PluginEventFamily } from "./PluginEventFamily.js";
import type { PluginId } from "./PluginId.js";
import type { PluginStage } from "./PluginStage.js";
import type { SecurityPluginMode } from "./SecurityPluginMode.js";

export interface EnvironmentPluginDescriptor {
  "description": string;
  "detail_capabilities": Array<PluginDetailCapability>;
  "diagnostic": boolean;
  "directions": Array<PluginDirection>;
  "event_families": Array<PluginEventFamily>;
  "fields": Array<EnvironmentPluginField>;
  "groups": Array<PluginControlGroup>;
  "id": PluginId;
  "limitations": Array<string>;
  "modes": Array<SecurityPluginMode>;
  "name": string;
  "required": boolean;
  "stage": PluginStage;
  "version": number;
}
