// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";
import type { EnvironmentSkillSummary } from "./EnvironmentSkillSummary.js";

export interface VmSkillListResponse {
  "discovery_fresh": boolean;
  "next_cursor"?: string | null;
  "owner_generation": string;
  "revision"?: null | EnvironmentRevisionReference;
  "skills": Array<EnvironmentSkillSummary>;
  "vm_id": string;
}
