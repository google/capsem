// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentCatalogSelections_EnvironmentCatalogPin } from "./EnvironmentCatalogSelections_EnvironmentCatalogPin.js";
import type { EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinition } from "./EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinition.js";
import type { EnvironmentPackages } from "./EnvironmentPackages.js";
import type { EnvironmentPlugins } from "./EnvironmentPlugins.js";
import type { EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRule } from "./EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRule.js";
import type { EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRule } from "./EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRule.js";
import type { EnvironmentSecretBinding } from "./EnvironmentSecretBinding.js";
import type { EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionImage } from "./EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionImage.js";
import type { EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionRepositoriesItem } from "./EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionRepositoriesItem.js";
import type { EnvironmentStartup } from "./EnvironmentStartup.js";

export interface EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition {
  "access"?: EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRule;
  "description"?: string;
  "detection"?: EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRule;
  "image"?: EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionImage;
  "mcps"?: EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinition;
  "name": string;
  "packages"?: EnvironmentPackages;
  "plugins"?: EnvironmentPlugins;
  "repositories"?: Array<EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionRepositoriesItem>;
  "secrets"?: Record<string, EnvironmentSecretBinding>;
  "skills"?: EnvironmentCatalogSelections_EnvironmentCatalogPin;
  "startup"?: null | EnvironmentStartup;
  "variables"?: Record<string, string>;
}
