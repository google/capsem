// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentCreateRequest} from "../models/EnvironmentCreateRequest.js";
import {EnvironmentSandboxTargetSchema} from "./EnvironmentSandboxTarget.js";
import {EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionSchema} from "./EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection.js";

export const EnvironmentCreateRequestSchema: z.ZodType<EnvironmentCreateRequest> = z.strictObject({
  "reason": z.string(),
  "request_id": z.uuid(),
  "requesting_sandbox": z.union([z.null(), z.lazy(() => EnvironmentSandboxTargetSchema)]).exactOptional(),
  "specification": z.lazy(() => EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionSchema),
});
