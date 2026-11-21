// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentPluginField} from "../models/EnvironmentPluginField.js";
import {EnvironmentPluginFieldDependencySchema} from "./EnvironmentPluginFieldDependency.js";
import {EnvironmentPluginFieldValueSchema} from "./EnvironmentPluginFieldValue.js";
import {WidgetSchema} from "./Widget.js";

export const EnvironmentPluginFieldSchema: z.ZodType<EnvironmentPluginField> = z.strictObject({
  "category": z.string(),
  "choices": z.array(z.string()),
  "default_value": z.lazy(() => EnvironmentPluginFieldValueSchema),
  "dependencies": z.array(z.lazy(() => EnvironmentPluginFieldDependencySchema)),
  "description": z.string(),
  "id": z.string(),
  "name": z.string(),
  "widget": z.union([z.null(), z.lazy(() => WidgetSchema)]).exactOptional(),
});
