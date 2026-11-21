// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentPluginFieldDependency } from "./EnvironmentPluginFieldDependency.js";
import type { EnvironmentPluginFieldValue } from "./EnvironmentPluginFieldValue.js";
import type { Widget } from "./Widget.js";

export interface EnvironmentPluginField {
  "category": string;
  "choices": Array<string>;
  "default_value": EnvironmentPluginFieldValue;
  "dependencies": Array<EnvironmentPluginFieldDependency>;
  "description": string;
  "id": string;
  "name": string;
  "widget"?: null | Widget;
}
