// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentTomlValidationResponse} from "../models/EnvironmentTomlValidationResponse.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";
import {EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionSchema} from "./EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection.js";

export const EnvironmentTomlValidationResponseSchema: z.ZodType<EnvironmentTomlValidationResponse> = z.strictObject({
  "base": z.lazy(() => EnvironmentRevisionReferenceSchema),
  "specification": z.lazy(() => EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionSchema),
});
