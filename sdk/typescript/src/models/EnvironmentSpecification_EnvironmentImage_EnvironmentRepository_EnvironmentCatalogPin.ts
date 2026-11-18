// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentCatalogSelections_EnvironmentCatalogPin } from "./EnvironmentCatalogSelections_EnvironmentCatalogPin.js";
import type { EnvironmentMcps_EnvironmentCatalogPin } from "./EnvironmentMcps_EnvironmentCatalogPin.js";
import type { EnvironmentPackages } from "./EnvironmentPackages.js";
import type { EnvironmentPlugins } from "./EnvironmentPlugins.js";
import type { EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRule } from "./EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRule.js";
import type { EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRule } from "./EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRule.js";
import type { EnvironmentSecretBinding } from "./EnvironmentSecretBinding.js";
import type { EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinImage } from "./EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinImage.js";
import type { EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinRepositoriesItem } from "./EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinRepositoriesItem.js";
import type { EnvironmentStartup } from "./EnvironmentStartup.js";

export interface EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin {
  "access"?: EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRule;
  "description"?: string;
  "detection"?: EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRule;
  "image"?: EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinImage;
  "mcps"?: EnvironmentMcps_EnvironmentCatalogPin;
  "name": string;
  "packages"?: EnvironmentPackages;
  "plugins"?: EnvironmentPlugins;
  "repositories"?: Array<EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinRepositoriesItem>;
  "secrets"?: Record<string, EnvironmentSecretBinding>;
  "skills"?: EnvironmentCatalogSelections_EnvironmentCatalogPin;
  "startup"?: null | EnvironmentStartup;
  "variables"?: Record<string, string>;
}
