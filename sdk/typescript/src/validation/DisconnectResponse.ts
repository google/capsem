// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {DisconnectResponse} from "../models/DisconnectResponse.js";
import {ConnectionSummarySchema} from "./ConnectionSummary.js";
import {LocalCredentialAuthoritySchema} from "./LocalCredentialAuthority.js";

export const DisconnectResponseSchema: z.ZodType<DisconnectResponse> = z.strictObject({
  "connection": z.lazy(() => ConnectionSummarySchema),
  "local_authority": z.lazy(() => LocalCredentialAuthoritySchema),
  "revoked_grant_count": z.int().min(0),
});
