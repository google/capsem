// Generated from Capsem OpenAPI. Do not edit.

import type { CatalogAvailability } from "./CatalogAvailability.js";
import type { CatalogKind } from "./CatalogKind.js";
import type { CatalogRefreshError } from "./CatalogRefreshError.js";
import type { CatalogSourceOrigin } from "./CatalogSourceOrigin.js";

export interface CatalogStatus {
  "availability": CatalogAvailability;
  "digest": string | null;
  "error": null | CatalogRefreshError;
  "kind": CatalogKind;
  "last_successful_refresh": number | null;
  "origin": null | CatalogSourceOrigin;
  "reference": string | null;
}
