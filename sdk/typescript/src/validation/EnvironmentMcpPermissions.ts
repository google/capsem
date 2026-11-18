// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentMcpPermissions} from "../models/EnvironmentMcpPermissions.js";
import {EnvironmentCapabilityDecisionSchema} from "./EnvironmentCapabilityDecision.js";

export const EnvironmentMcpPermissionsSchema: z.ZodType<EnvironmentMcpPermissions> = z.strictObject({
  "default": z.lazy(() => EnvironmentCapabilityDecisionSchema).exactOptional(),
  "enabled": z.boolean().exactOptional(),
  "tools": z.record(z.string(), z.lazy(() => EnvironmentCapabilityDecisionSchema)).exactOptional(),
});
