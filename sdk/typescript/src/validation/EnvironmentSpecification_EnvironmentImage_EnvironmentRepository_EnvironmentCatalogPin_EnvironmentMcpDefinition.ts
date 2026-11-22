// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition} from "../models/EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition.js";
import {EnvironmentCatalogSelections_EnvironmentCatalogPinSchema} from "./EnvironmentCatalogSelections_EnvironmentCatalogPin.js";
import {EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionSchema} from "./EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinition.js";
import {EnvironmentPackagesSchema} from "./EnvironmentPackages.js";
import {EnvironmentPluginsSchema} from "./EnvironmentPlugins.js";
import {EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleSchema} from "./EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRule.js";
import {EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRuleSchema} from "./EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRule.js";
import {EnvironmentSecretBindingSchema} from "./EnvironmentSecretBinding.js";
import {EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionImageSchema} from "./EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionImage.js";
import {EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionRepositoriesItemSchema} from "./EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionRepositoriesItem.js";
import {EnvironmentStartupSchema} from "./EnvironmentStartup.js";

export const EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionSchema: z.ZodType<EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition> = z.strictObject({
  "access": z.lazy(() => EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleSchema).exactOptional(),
  "description": z.string().exactOptional(),
  "detection": z.lazy(() => EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRuleSchema).exactOptional(),
  "image": z.lazy(() => EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionImageSchema).exactOptional(),
  "mcps": z.lazy(() => EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionSchema).exactOptional(),
  "name": z.string(),
  "packages": z.lazy(() => EnvironmentPackagesSchema).exactOptional(),
  "plugins": z.lazy(() => EnvironmentPluginsSchema).exactOptional(),
  "repositories": z.array(z.lazy(() => EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionRepositoriesItemSchema)).exactOptional(),
  "secrets": z.record(z.string(), z.lazy(() => EnvironmentSecretBindingSchema)).exactOptional(),
  "skills": z.lazy(() => EnvironmentCatalogSelections_EnvironmentCatalogPinSchema).exactOptional(),
  "startup": z.union([z.null(), z.lazy(() => EnvironmentStartupSchema)]).exactOptional(),
  "variables": z.record(z.string(), z.string()).exactOptional(),
});
