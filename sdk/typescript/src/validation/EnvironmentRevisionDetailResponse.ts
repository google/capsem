// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentRevisionDetailResponse} from "../models/EnvironmentRevisionDetailResponse.js";
import {EnvironmentRevisionSummarySchema} from "./EnvironmentRevisionSummary.js";
import {EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionSchema} from "./EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition.js";
import {RequestReviewAvailabilitySchema} from "./RequestReviewAvailability.js";

export const EnvironmentRevisionDetailResponseSchema: z.ZodType<EnvironmentRevisionDetailResponse> = z.strictObject({
  "availability": z.lazy(() => RequestReviewAvailabilitySchema),
  "canonical_toml": z.string().nullable().exactOptional(),
  "partial_specification": z.union([z.null(), z.lazy(() => EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionSchema)]).exactOptional(),
  "purged_variables": z.array(z.string()),
  "summary": z.lazy(() => EnvironmentRevisionSummarySchema),
});
