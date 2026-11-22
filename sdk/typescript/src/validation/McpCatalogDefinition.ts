// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpCatalogDefinition} from "../models/McpCatalogDefinition.js";
import {McpCatalogDefinitionRemoteSchema} from "./McpCatalogDefinitionRemote.js";
import {McpCatalogDefinitionSandboxSchema} from "./McpCatalogDefinitionSandbox.js";

export const McpCatalogDefinitionSchema: z.ZodType<McpCatalogDefinition> = z.union([z.lazy(() => McpCatalogDefinitionRemoteSchema), z.lazy(() => McpCatalogDefinitionSandboxSchema)]);
