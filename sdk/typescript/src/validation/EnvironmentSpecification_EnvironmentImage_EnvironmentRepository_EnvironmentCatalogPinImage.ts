// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinImage} from "../models/EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinImage.js";
import {EnvironmentImageUpdatePolicySchema} from "./EnvironmentImageUpdatePolicy.js";

export const EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinImageSchema: z.ZodType<EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinImage> = z.strictObject({
  "reference": z.string(),
  "resolved_digest": z.string(),
  "update_policy": z.lazy(() => EnvironmentImageUpdatePolicySchema).exactOptional(),
});
