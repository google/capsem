// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionImage} from "../models/EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionImage.js";
import {EnvironmentImageUpdatePolicySchema} from "./EnvironmentImageUpdatePolicy.js";

export const EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionImageSchema: z.ZodType<EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionImage> = z.strictObject({
  "reference": z.string(),
  "update_policy": z.lazy(() => EnvironmentImageUpdatePolicySchema).exactOptional(),
});
