// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ConnectionSummary} from "../models/ConnectionSummary.js";
import {ConnectionProviderSchema} from "./ConnectionProvider.js";
import {ConnectionStateSchema} from "./ConnectionState.js";
import {CredentialErrorCodeSchema} from "./CredentialErrorCode.js";
import {ProviderRevocationStateSchema} from "./ProviderRevocationState.js";

export const ConnectionSummarySchema: z.ZodType<ConnectionSummary> = z.strictObject({
  "account_label": z.string().nullable(),
  "available_capabilities": z.array(z.string()),
  "created_at": z.iso.datetime({offset: true}),
  "error_code": z.union([z.null(), z.lazy(() => CredentialErrorCodeSchema)]),
  "granted_scopes": z.array(z.string()),
  "id": z.string(),
  "last_refresh_at": z.iso.datetime({offset: true}).nullable(),
  "provider": z.lazy(() => ConnectionProviderSchema),
  "provider_revocation": z.lazy(() => ProviderRevocationStateSchema),
  "revision": z.int().min(0).max(9007199254740991),
  "scope_profile_id": z.string(),
  "state": z.lazy(() => ConnectionStateSchema),
  "updated_at": z.iso.datetime({offset: true}),
});
