// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RuleCatalogDefinition} from "../models/RuleCatalogDefinition.js";
import {RuleCatalogDefinitionAccessSchema} from "./RuleCatalogDefinitionAccess.js";
import {RuleCatalogDefinitionDetectionSchema} from "./RuleCatalogDefinitionDetection.js";

export const RuleCatalogDefinitionSchema: z.ZodType<RuleCatalogDefinition> = z.union([z.lazy(() => RuleCatalogDefinitionAccessSchema), z.lazy(() => RuleCatalogDefinitionDetectionSchema)]);
