// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentSandboxTarget } from "./EnvironmentSandboxTarget.js";
import type { EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInput } from "./EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInput.js";

export interface EnvironmentChangeRequest {
  "base_revision_id": string;
  "reason": string;
  "request_id": string;
  "requesting_sandbox"?: null | EnvironmentSandboxTarget;
  "specification": EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInput;
}
