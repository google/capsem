// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {AuthorizationTransaction} from "../models/AuthorizationTransaction.js";
import {CredentialErrorCodeSchema} from "./CredentialErrorCode.js";
import {TransactionStateSchema} from "./TransactionState.js";

export const AuthorizationTransactionSchema: z.ZodType<AuthorizationTransaction> = z.strictObject({
  "connection_id": z.string().nullable(),
  "error_code": z.union([z.null(), z.lazy(() => CredentialErrorCodeSchema)]),
  "expires_at": z.iso.datetime({offset: true}),
  "id": z.string(),
  "state": z.lazy(() => TransactionStateSchema),
});
