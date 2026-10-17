// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CredentialInjectResponse} from "../models/CredentialInjectResponse.js";
import {CredentialStorageSchema} from "./CredentialStorage.js";

export const CredentialInjectResponseSchema: z.ZodType<CredentialInjectResponse> = z.strictObject({
  "credential_ref": z.string(),
  "storage": z.lazy(() => CredentialStorageSchema),
});
