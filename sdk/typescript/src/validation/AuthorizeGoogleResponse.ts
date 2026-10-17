// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {AuthorizeGoogleResponse} from "../models/AuthorizeGoogleResponse.js";
import {AuthorizationTransactionSchema} from "./AuthorizationTransaction.js";

export const AuthorizeGoogleResponseSchema: z.ZodType<AuthorizeGoogleResponse> = z.strictObject({
  "authorization_url": z.string().nullable(),
  "transaction": z.lazy(() => AuthorizationTransactionSchema),
});
