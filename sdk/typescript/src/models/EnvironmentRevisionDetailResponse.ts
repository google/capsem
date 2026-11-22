// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentRevisionSummary } from "./EnvironmentRevisionSummary.js";
import type { EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition } from "./EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition.js";
import type { RequestReviewAvailability } from "./RequestReviewAvailability.js";

export interface EnvironmentRevisionDetailResponse {
  "availability": RequestReviewAvailability;
  "canonical_toml"?: string | null;
  "partial_specification"?: null | EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition;
  "purged_variables": Array<string>;
  "summary": EnvironmentRevisionSummary;
}
