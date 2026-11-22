// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RuleCatalogDefinitionAccess} from "../models/RuleCatalogDefinitionAccess.js";
import {EnvironmentAccessRuleSchema} from "./EnvironmentAccessRule.js";

export const RuleCatalogDefinitionAccessSchema: z.ZodType<RuleCatalogDefinitionAccess> = z.object({
  "kind": z.literal("access"),
  "rule": z.lazy(() => EnvironmentAccessRuleSchema),
});
