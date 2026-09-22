// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CapturedContent} from "../models/CapturedContent.js";
import {JsonContentSchema} from "./JsonContent.js";
import {RawContentSchema} from "./RawContent.js";
import {TextContentSchema} from "./TextContent.js";

export const CapturedContentSchema: z.ZodType<CapturedContent> = z.union([z.lazy(() => JsonContentSchema), z.lazy(() => TextContentSchema), z.lazy(() => RawContentSchema)]);
