// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputRepositoriesItem} from "../models/EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputRepositoriesItem.js";

export const EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputRepositoriesItemSchema: z.ZodType<EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputRepositoriesItem> = z.strictObject({
  "destination": z.string(),
  "ref": z.string(),
  "url": z.string(),
});
