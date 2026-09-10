// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {InteractionRequest} from "../models/InteractionRequest.js";
import {CapturedPayloadSchema} from "./CapturedPayload.js";
import {InteractionRequestKindSchema} from "./InteractionRequestKind.js";

export const InteractionRequestSchema: z.ZodType<InteractionRequest> = z.strictObject({
  "kind": z.lazy(() => InteractionRequestKindSchema),
  "payload": z.union([z.null(), z.lazy(() => CapturedPayloadSchema)]).exactOptional(),
});
