// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection} from "../models/EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection.js";
import {EnvironmentCatalogSelections_EnvironmentCatalogSelectionSchema} from "./EnvironmentCatalogSelections_EnvironmentCatalogSelection.js";
import {EnvironmentMcps_EnvironmentCatalogSelectionSchema} from "./EnvironmentMcps_EnvironmentCatalogSelection.js";
import {EnvironmentPackagesSchema} from "./EnvironmentPackages.js";
import {EnvironmentPluginsSchema} from "./EnvironmentPlugins.js";
import {EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleSchema} from "./EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRule.js";
import {EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRuleSchema} from "./EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRule.js";
import {EnvironmentSecretBindingSchema} from "./EnvironmentSecretBinding.js";
import {EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionImageSchema} from "./EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionImage.js";
import {EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionRepositoriesItemSchema} from "./EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionRepositoriesItem.js";
import {EnvironmentStartupSchema} from "./EnvironmentStartup.js";

export const EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionSchema: z.ZodType<EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection> = z.strictObject({
  "access": z.lazy(() => EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleSchema).exactOptional(),
  "description": z.string().exactOptional(),
  "detection": z.lazy(() => EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRuleSchema).exactOptional(),
  "image": z.lazy(() => EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionImageSchema).exactOptional(),
  "mcps": z.lazy(() => EnvironmentMcps_EnvironmentCatalogSelectionSchema).exactOptional(),
  "name": z.string(),
  "packages": z.lazy(() => EnvironmentPackagesSchema).exactOptional(),
  "plugins": z.lazy(() => EnvironmentPluginsSchema).exactOptional(),
  "repositories": z.array(z.lazy(() => EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionRepositoriesItemSchema)).exactOptional(),
  "secrets": z.record(z.string(), z.lazy(() => EnvironmentSecretBindingSchema)).exactOptional(),
  "skills": z.lazy(() => EnvironmentCatalogSelections_EnvironmentCatalogSelectionSchema).exactOptional(),
  "startup": z.union([z.null(), z.lazy(() => EnvironmentStartupSchema)]).exactOptional(),
  "variables": z.record(z.string(), z.string()).exactOptional(),
});
