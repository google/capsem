// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentMcpDefinition} from "../models/EnvironmentMcpDefinition.js";
import {EnvironmentMcpDefinitionRemoteSchema} from "./EnvironmentMcpDefinitionRemote.js";
import {EnvironmentMcpDefinitionSandboxSchema} from "./EnvironmentMcpDefinitionSandbox.js";

export const EnvironmentMcpDefinitionSchema: z.ZodType<EnvironmentMcpDefinition> = z.union([z.lazy(() => EnvironmentMcpDefinitionRemoteSchema), z.lazy(() => EnvironmentMcpDefinitionSandboxSchema)]);
