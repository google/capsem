// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";
import type { EnvironmentSkillSummary } from "./EnvironmentSkillSummary.js";

export interface EnvironmentSkillListResponse {
  "discovery_fresh": boolean;
  "next_cursor"?: string | null;
  "revision": EnvironmentRevisionReference;
  "skills": Array<EnvironmentSkillSummary>;
}
