// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionRepositoriesItem} from "../models/EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionRepositoriesItem.js";

export const EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionRepositoriesItemSchema: z.ZodType<EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionRepositoriesItem> = z.strictObject({
  "destination": z.string(),
  "ref": z.string(),
  "url": z.string(),
});
