// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionImage} from "../models/EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionImage.js";
import {EnvironmentImageUpdatePolicySchema} from "./EnvironmentImageUpdatePolicy.js";

export const EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionImageSchema: z.ZodType<EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionImage> = z.strictObject({
  "reference": z.string(),
  "resolved_digest": z.string(),
  "update_policy": z.lazy(() => EnvironmentImageUpdatePolicySchema).exactOptional(),
});
