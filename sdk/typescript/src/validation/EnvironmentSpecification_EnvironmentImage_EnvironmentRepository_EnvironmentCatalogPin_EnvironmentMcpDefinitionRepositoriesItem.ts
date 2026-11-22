// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionRepositoriesItem} from "../models/EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionRepositoriesItem.js";

export const EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionRepositoriesItemSchema: z.ZodType<EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionRepositoriesItem> = z.strictObject({
  "destination": z.string(),
  "ref": z.string(),
  "resolved_commit": z.string(),
  "url": z.string(),
});
