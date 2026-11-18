// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SandboxEnvironment} from "../models/SandboxEnvironment.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";

export const SandboxEnvironmentSchema: z.ZodType<SandboxEnvironment> = z.strictObject({
  "boot": z.union([z.null(), z.lazy(() => EnvironmentRevisionReferenceSchema)]).exactOptional(),
  "desired": z.lazy(() => EnvironmentRevisionReferenceSchema),
  "live_policy": z.union([z.null(), z.lazy(() => EnvironmentRevisionReferenceSchema)]).exactOptional(),
  "owner_generation": z.string().nullable().exactOptional(),
  "pending_restart": z.boolean().nullable().exactOptional(),
  "policy_digest": z.string().nullable().exactOptional(),
});
