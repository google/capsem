// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentPluginDescriptor } from "./EnvironmentPluginDescriptor.js";
import type { EnvironmentPluginDirectionState } from "./EnvironmentPluginDirectionState.js";
import type { EnvironmentPluginFieldValue } from "./EnvironmentPluginFieldValue.js";
import type { EnvironmentPluginSource } from "./EnvironmentPluginSource.js";

export interface EnvironmentPluginEntry {
  "configured": Record<string, EnvironmentPluginFieldValue>;
  "descriptor": EnvironmentPluginDescriptor;
  "directions": Array<EnvironmentPluginDirectionState>;
  "effective": Record<string, EnvironmentPluginFieldValue>;
  "locked": boolean;
  "source": EnvironmentPluginSource;
}
