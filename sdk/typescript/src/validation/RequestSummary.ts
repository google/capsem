// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RequestSummary} from "../models/RequestSummary.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";
import {EnvironmentSandboxTargetSchema} from "./EnvironmentSandboxTarget.js";
import {RequestApplicationStateSchema} from "./RequestApplicationState.js";
import {RequestKindSchema} from "./RequestKind.js";
import {RequestStateSchema} from "./RequestState.js";

export const RequestSummarySchema: z.ZodType<RequestSummary> = z.strictObject({
  "actor": z.string(),
  "application": z.lazy(() => RequestApplicationStateSchema),
  "base": z.union([z.null(), z.lazy(() => EnvironmentRevisionReferenceSchema)]).exactOptional(),
  "decision": z.lazy(() => RequestStateSchema),
  "environment_id": z.uuid(),
  "expires_at_unix_ms": z.int(),
  "kind": z.lazy(() => RequestKindSchema),
  "reason": z.string(),
  "request_id": z.uuid(),
  "requester": z.union([z.null(), z.lazy(() => EnvironmentSandboxTargetSchema)]).exactOptional(),
  "result": z.union([z.null(), z.lazy(() => EnvironmentRevisionReferenceSchema)]).exactOptional(),
  "source": z.union([z.null(), z.lazy(() => EnvironmentRevisionReferenceSchema)]).exactOptional(),
  "timestamp_unix_ms": z.int(),
});
