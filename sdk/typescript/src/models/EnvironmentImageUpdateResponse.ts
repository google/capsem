// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentImage } from "./EnvironmentImage.js";
import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";

export interface EnvironmentImageUpdateResponse {
  "available"?: null | EnvironmentImage;
  "current"?: null | EnvironmentImage;
  "environment": EnvironmentRevisionReference;
}
