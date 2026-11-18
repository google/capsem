// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinRepositoriesItem} from "../models/EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinRepositoriesItem.js";

export const EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinRepositoriesItemSchema: z.ZodType<EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinRepositoriesItem> = z.strictObject({
  "destination": z.string(),
  "ref": z.string(),
  "resolved_commit": z.string(),
  "url": z.string(),
});
