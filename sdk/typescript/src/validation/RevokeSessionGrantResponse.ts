// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RevokeSessionGrantResponse} from "../models/RevokeSessionGrantResponse.js";
import {RevokedCredentialAuthoritySchema} from "./RevokedCredentialAuthority.js";
import {SessionGrantSummarySchema} from "./SessionGrantSummary.js";

export const RevokeSessionGrantResponseSchema: z.ZodType<RevokeSessionGrantResponse> = z.strictObject({
  "grant": z.lazy(() => SessionGrantSummarySchema),
  "local_authority": z.lazy(() => RevokedCredentialAuthoritySchema),
});
