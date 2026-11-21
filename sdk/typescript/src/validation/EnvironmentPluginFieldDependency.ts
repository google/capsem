// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentPluginFieldDependency} from "../models/EnvironmentPluginFieldDependency.js";
import {EnvironmentPluginFieldValueSchema} from "./EnvironmentPluginFieldValue.js";

export const EnvironmentPluginFieldDependencySchema: z.ZodType<EnvironmentPluginFieldDependency> = z.strictObject({
  "field": z.string(),
  "not_equals": z.lazy(() => EnvironmentPluginFieldValueSchema),
});
