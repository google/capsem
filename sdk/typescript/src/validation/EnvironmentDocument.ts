// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentDocument} from "../models/EnvironmentDocument.js";
import {EnvironmentIdentitySchema} from "./EnvironmentIdentity.js";
import {EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionSchema} from "./EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition.js";

export const EnvironmentDocumentSchema: z.ZodType<EnvironmentDocument> = z.strictObject({
  "identity": z.lazy(() => EnvironmentIdentitySchema),
  "specification": z.lazy(() => EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionSchema),
});
