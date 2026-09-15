// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {InteractionMessage} from "../models/InteractionMessage.js";
import {InteractionBlockSchema} from "./InteractionBlock.js";
import {InteractionMessageKindSchema} from "./InteractionMessageKind.js";
import {InteractionRoleSchema} from "./InteractionRole.js";

export const InteractionMessageSchema: z.ZodType<InteractionMessage> = z.strictObject({
  "blocks": z.array(z.lazy(() => InteractionBlockSchema)),
  "kind": z.lazy(() => InteractionMessageKindSchema),
  "role": z.lazy(() => InteractionRoleSchema),
});
