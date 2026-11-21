// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentChangeIntent} from "../models/EnvironmentChangeIntent.js";
import {EnvironmentArchiveRequestSchema} from "./EnvironmentArchiveRequest.js";
import {EnvironmentChangeRequestSchema} from "./EnvironmentChangeRequest.js";

export const EnvironmentChangeIntentSchema: z.ZodType<EnvironmentChangeIntent> = z.union([z.lazy(() => EnvironmentChangeRequestSchema), z.lazy(() => EnvironmentArchiveRequestSchema)]);
