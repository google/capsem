// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentCatalogSelections_EnvironmentCatalogSelection } from "./EnvironmentCatalogSelections_EnvironmentCatalogSelection.js";
import type { EnvironmentMcps_EnvironmentCatalogSelection } from "./EnvironmentMcps_EnvironmentCatalogSelection.js";
import type { EnvironmentPackages } from "./EnvironmentPackages.js";
import type { EnvironmentPlugins } from "./EnvironmentPlugins.js";
import type { EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRule } from "./EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRule.js";
import type { EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRule } from "./EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRule.js";
import type { EnvironmentSecretBinding } from "./EnvironmentSecretBinding.js";
import type { EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionImage } from "./EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionImage.js";
import type { EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionRepositoriesItem } from "./EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionRepositoriesItem.js";
import type { EnvironmentStartup } from "./EnvironmentStartup.js";

export interface EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection {
  "access"?: EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRule;
  "description"?: string;
  "detection"?: EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRule;
  "image"?: EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionImage;
  "mcps"?: EnvironmentMcps_EnvironmentCatalogSelection;
  "name": string;
  "packages"?: EnvironmentPackages;
  "plugins"?: EnvironmentPlugins;
  "repositories"?: Array<EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionRepositoriesItem>;
  "secrets"?: Record<string, EnvironmentSecretBinding>;
  "skills"?: EnvironmentCatalogSelections_EnvironmentCatalogSelection;
  "startup"?: null | EnvironmentStartup;
  "variables"?: Record<string, string>;
}
