// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ManagedSessionClaimResponse} from "../models/ManagedSessionClaimResponse.js";
import {ManagedSessionFailureSchema} from "./ManagedSessionFailure.js";
import {ManagedSessionOwnershipSchema} from "./ManagedSessionOwnership.js";
import {ManagedSessionStateSchema} from "./ManagedSessionState.js";
import {ProvisionResponseSchema} from "./ProvisionResponse.js";

export const ManagedSessionClaimResponseSchema: z.ZodType<ManagedSessionClaimResponse> = z.strictObject({
  "error": z.union([z.null(), z.lazy(() => ManagedSessionFailureSchema)]).exactOptional(),
  "ownership": z.lazy(() => ManagedSessionOwnershipSchema),
  "session": z.union([z.null(), z.lazy(() => ProvisionResponseSchema)]).exactOptional(),
  "state": z.lazy(() => ManagedSessionStateSchema),
});
