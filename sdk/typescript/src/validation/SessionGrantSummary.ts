// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SessionGrantSummary} from "../models/SessionGrantSummary.js";
import {GrantStateSchema} from "./GrantState.js";

export const SessionGrantSummarySchema: z.ZodType<SessionGrantSummary> = z.strictObject({
  "capability_ids": z.array(z.string()),
  "connection_id": z.string(),
  "created_at": z.iso.datetime({offset: true}),
  "expires_at": z.iso.datetime({offset: true}),
  "id": z.string(),
  "owner_generation": z.string(),
  "revision": z.int().min(0).max(9007199254740991),
  "revoked_at": z.iso.datetime({offset: true}).nullable(),
  "runtime_bound": z.boolean(),
  "session_id": z.string(),
  "state": z.lazy(() => GrantStateSchema),
});
