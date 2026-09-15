// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {InteractionContent} from "../models/InteractionContent.js";
import {InteractionMessageSchema} from "./InteractionMessage.js";
import {InteractionRequestSchema} from "./InteractionRequest.js";
import {InteractionToolCallSchema} from "./InteractionToolCall.js";
import {InteractionToolResultSchema} from "./InteractionToolResult.js";

export const InteractionContentSchema: z.ZodType<InteractionContent> = z.union([z.lazy(() => InteractionRequestSchema), z.lazy(() => InteractionMessageSchema), z.lazy(() => InteractionToolCallSchema), z.lazy(() => InteractionToolResultSchema)]);
