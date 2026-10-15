// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {AuthorizeGoogleRequest} from "../models/AuthorizeGoogleRequest.js";

export const AuthorizeGoogleRequestSchema: z.ZodType<AuthorizeGoogleRequest> = z.strictObject({
  "reconnect_connection_id": z.string().nullable(),
  "request_id": z.uuid(),
  "scope_profile_id": z.string(),
});
