// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentPluginEntry } from "./EnvironmentPluginEntry.js";
import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";

export interface EnvironmentPluginListResponse {
  "plugins": Array<EnvironmentPluginEntry>;
  "revision": null | EnvironmentRevisionReference;
}
