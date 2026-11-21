// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentRevisionSummary } from "./EnvironmentRevisionSummary.js";
import type { EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin } from "./EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin.js";
import type { RequestReviewAvailability } from "./RequestReviewAvailability.js";

export interface EnvironmentRevisionDetailResponse {
  "availability": RequestReviewAvailability;
  "canonical_toml"?: string | null;
  "partial_specification"?: null | EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin;
  "purged_variables": Array<string>;
  "summary": EnvironmentRevisionSummary;
}
