// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CreateSessionGrantRequest} from "../models/CreateSessionGrantRequest.js";

export const CreateSessionGrantRequestSchema: z.ZodType<CreateSessionGrantRequest> = z.strictObject({
  "capability_ids": z.array(z.string()),
  "connection_id": z.string(),
  "expected_owner_generation": z.uuid(),
  "expires_at": z.iso.datetime({offset: true}).nullable(),
  "request_id": z.uuid(),
});
