// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentChangeRequest} from "../models/EnvironmentChangeRequest.js";
import {EnvironmentSandboxTargetSchema} from "./EnvironmentSandboxTarget.js";
import {EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputSchema} from "./EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInput.js";

export const EnvironmentChangeRequestSchema: z.ZodType<EnvironmentChangeRequest> = z.strictObject({
  "base_revision_id": z.uuid(),
  "reason": z.string(),
  "request_id": z.uuid(),
  "requesting_sandbox": z.union([z.null(), z.lazy(() => EnvironmentSandboxTargetSchema)]).exactOptional(),
  "specification": z.lazy(() => EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputSchema),
});
