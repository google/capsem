// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentReviewContent} from "../models/EnvironmentReviewContent.js";
import {EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinSchema} from "./EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin.js";
import {RequestReviewAvailabilitySchema} from "./RequestReviewAvailability.js";

export const EnvironmentReviewContentSchema: z.ZodType<EnvironmentReviewContent> = z.strictObject({
  "availability": z.lazy(() => RequestReviewAvailabilitySchema),
  "purged_variables": z.array(z.string()),
  "specification": z.union([z.null(), z.lazy(() => EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinSchema)]).exactOptional(),
});
