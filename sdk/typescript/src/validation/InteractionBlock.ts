// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {InteractionBlock} from "../models/InteractionBlock.js";
import {CapturedPayloadSchema} from "./CapturedPayload.js";
import {InteractionBlockKindSchema} from "./InteractionBlockKind.js";

export const InteractionBlockSchema: z.ZodType<InteractionBlock> = z.object({
  "kind": z.lazy(() => InteractionBlockKindSchema),
  "payload": z.union([z.null(), z.lazy(() => CapturedPayloadSchema)]).exactOptional(),
});
