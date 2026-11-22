// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputImage} from "../models/EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputImage.js";
import {EnvironmentImageUpdatePolicySchema} from "./EnvironmentImageUpdatePolicy.js";

export const EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputImageSchema: z.ZodType<EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputImage> = z.strictObject({
  "reference": z.string(),
  "update_policy": z.lazy(() => EnvironmentImageUpdatePolicySchema).exactOptional(),
});
