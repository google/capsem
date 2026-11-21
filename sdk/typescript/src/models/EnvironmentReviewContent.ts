// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin } from "./EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin.js";
import type { RequestReviewAvailability } from "./RequestReviewAvailability.js";

export interface EnvironmentReviewContent {
  "availability": RequestReviewAvailability;
  "purged_variables": Array<string>;
  "specification"?: null | EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin;
}
