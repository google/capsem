// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentMutationResponse} from "../models/EnvironmentMutationResponse.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";
import {RequestApplicationStateSchema} from "./RequestApplicationState.js";
import {RequestStateSchema} from "./RequestState.js";

export const EnvironmentMutationResponseSchema: z.ZodType<EnvironmentMutationResponse> = z.strictObject({
  "application": z.lazy(() => RequestApplicationStateSchema),
  "application_error": z.string().nullable().exactOptional(),
  "decision": z.lazy(() => RequestStateSchema),
  "environment_id": z.uuid(),
  "request_id": z.uuid(),
  "result": z.union([z.null(), z.lazy(() => EnvironmentRevisionReferenceSchema)]).exactOptional(),
});
