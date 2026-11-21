// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentPluginListResponse} from "../models/EnvironmentPluginListResponse.js";
import {EnvironmentPluginEntrySchema} from "./EnvironmentPluginEntry.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";

export const EnvironmentPluginListResponseSchema: z.ZodType<EnvironmentPluginListResponse> = z.strictObject({
  "plugins": z.array(z.lazy(() => EnvironmentPluginEntrySchema)),
  "revision": z.union([z.null(), z.lazy(() => EnvironmentRevisionReferenceSchema)]),
});
