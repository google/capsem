// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin} from "../models/EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin.js";
import {EnvironmentCatalogSelections_EnvironmentCatalogPinSchema} from "./EnvironmentCatalogSelections_EnvironmentCatalogPin.js";
import {EnvironmentMcps_EnvironmentCatalogPinSchema} from "./EnvironmentMcps_EnvironmentCatalogPin.js";
import {EnvironmentPackagesSchema} from "./EnvironmentPackages.js";
import {EnvironmentPluginsSchema} from "./EnvironmentPlugins.js";
import {EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleSchema} from "./EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRule.js";
import {EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRuleSchema} from "./EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRule.js";
import {EnvironmentSecretBindingSchema} from "./EnvironmentSecretBinding.js";
import {EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinImageSchema} from "./EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinImage.js";
import {EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinRepositoriesItemSchema} from "./EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinRepositoriesItem.js";
import {EnvironmentStartupSchema} from "./EnvironmentStartup.js";

export const EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinSchema: z.ZodType<EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin> = z.strictObject({
  "access": z.lazy(() => EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleSchema).exactOptional(),
  "description": z.string().exactOptional(),
  "detection": z.lazy(() => EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRuleSchema).exactOptional(),
  "image": z.lazy(() => EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinImageSchema).exactOptional(),
  "mcps": z.lazy(() => EnvironmentMcps_EnvironmentCatalogPinSchema).exactOptional(),
  "name": z.string(),
  "packages": z.lazy(() => EnvironmentPackagesSchema).exactOptional(),
  "plugins": z.lazy(() => EnvironmentPluginsSchema).exactOptional(),
  "repositories": z.array(z.lazy(() => EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinRepositoriesItemSchema)).exactOptional(),
  "secrets": z.record(z.string(), z.lazy(() => EnvironmentSecretBindingSchema)).exactOptional(),
  "skills": z.lazy(() => EnvironmentCatalogSelections_EnvironmentCatalogPinSchema).exactOptional(),
  "startup": z.union([z.null(), z.lazy(() => EnvironmentStartupSchema)]).exactOptional(),
  "variables": z.record(z.string(), z.string()).exactOptional(),
});
