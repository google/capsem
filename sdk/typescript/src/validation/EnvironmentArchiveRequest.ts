// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentArchiveRequest} from "../models/EnvironmentArchiveRequest.js";
import {EnvironmentArchiveActionSchema} from "./EnvironmentArchiveAction.js";

export const EnvironmentArchiveRequestSchema: z.ZodType<EnvironmentArchiveRequest> = z.strictObject({
  "action": z.lazy(() => EnvironmentArchiveActionSchema),
  "base_revision_id": z.uuid(),
  "reason": z.string(),
  "request_id": z.uuid(),
});
