// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RequestTargetApplication} from "../models/RequestTargetApplication.js";
import {EnvironmentSandboxTargetSchema} from "./EnvironmentSandboxTarget.js";
import {RequestApplicationCompositionSchema} from "./RequestApplicationComposition.js";
import {RequestApplicationStateSchema} from "./RequestApplicationState.js";

export const RequestTargetApplicationSchema: z.ZodType<RequestTargetApplication> = z.strictObject({
  "composition": z.union([z.null(), z.lazy(() => RequestApplicationCompositionSchema)]).exactOptional(),
  "failure_reason": z.string().nullable().exactOptional(),
  "state": z.lazy(() => RequestApplicationStateSchema),
  "target": z.lazy(() => EnvironmentSandboxTargetSchema),
  "timestamp_unix_ms": z.int().nullable().exactOptional(),
});
