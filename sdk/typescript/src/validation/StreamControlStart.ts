// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {StreamControlStart} from "../models/StreamControlStart.js";
import {StreamKindSchema} from "./StreamKind.js";

export const StreamControlStartSchema: z.ZodType<StreamControlStart> = z.object({
  "command": z.string().nullable().exactOptional(),
  "kind": z.lazy(() => StreamKindSchema),
  "type": z.literal("start"),
});
