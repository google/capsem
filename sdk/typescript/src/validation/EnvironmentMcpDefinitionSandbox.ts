// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentMcpDefinitionSandbox} from "../models/EnvironmentMcpDefinitionSandbox.js";
import {EnvironmentSecretBindingSchema} from "./EnvironmentSecretBinding.js";

export const EnvironmentMcpDefinitionSandboxSchema: z.ZodType<EnvironmentMcpDefinitionSandbox> = z.object({
  "args": z.array(z.string()).exactOptional(),
  "executable": z.string(),
  "kind": z.literal("sandbox"),
  "secrets": z.record(z.string(), z.lazy(() => EnvironmentSecretBindingSchema)).exactOptional(),
  "variables": z.record(z.string(), z.string()).exactOptional(),
  "workdir": z.string().nullable().exactOptional(),
});
