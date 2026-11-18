// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentDocument} from "../models/EnvironmentDocument.js";
import {EnvironmentIdentitySchema} from "./EnvironmentIdentity.js";
import {EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinSchema} from "./EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin.js";

export const EnvironmentDocumentSchema: z.ZodType<EnvironmentDocument> = z.strictObject({
  "identity": z.lazy(() => EnvironmentIdentitySchema),
  "specification": z.lazy(() => EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinSchema),
});
