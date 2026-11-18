// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentSandboxTarget } from "./EnvironmentSandboxTarget.js";
import type { EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection } from "./EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection.js";

export interface EnvironmentCreateRequest {
  "reason": string;
  "request_id": string;
  "requesting_sandbox"?: null | EnvironmentSandboxTarget;
  "specification": EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection;
}
