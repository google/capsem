// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentPluginEntry} from "../models/EnvironmentPluginEntry.js";
import {EnvironmentPluginDescriptorSchema} from "./EnvironmentPluginDescriptor.js";
import {EnvironmentPluginDirectionStateSchema} from "./EnvironmentPluginDirectionState.js";
import {EnvironmentPluginFieldValueSchema} from "./EnvironmentPluginFieldValue.js";
import {EnvironmentPluginSourceSchema} from "./EnvironmentPluginSource.js";

export const EnvironmentPluginEntrySchema: z.ZodType<EnvironmentPluginEntry> = z.strictObject({
  "configured": z.record(z.string(), z.lazy(() => EnvironmentPluginFieldValueSchema)),
  "descriptor": z.lazy(() => EnvironmentPluginDescriptorSchema),
  "directions": z.array(z.lazy(() => EnvironmentPluginDirectionStateSchema)),
  "effective": z.record(z.string(), z.lazy(() => EnvironmentPluginFieldValueSchema)),
  "locked": z.boolean(),
  "source": z.lazy(() => EnvironmentPluginSourceSchema),
});
