// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PreviewSessionsRevokedResponse} from "../models/PreviewSessionsRevokedResponse.js";

export const PreviewSessionsRevokedResponseSchema: z.ZodType<PreviewSessionsRevokedResponse> = z.object({
  "revoked": z.int().min(0),
});
