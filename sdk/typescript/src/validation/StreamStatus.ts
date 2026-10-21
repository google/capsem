// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {StreamStatus} from "../models/StreamStatus.js";
import {StreamStatusErrorSchema} from "./StreamStatusError.js";
import {StreamStatusExitSchema} from "./StreamStatusExit.js";
import {StreamStatusStartedSchema} from "./StreamStatusStarted.js";
import {StreamStatusTerminalSchema} from "./StreamStatusTerminal.js";

export const StreamStatusSchema: z.ZodType<StreamStatus> = z.union([z.lazy(() => StreamStatusStartedSchema), z.lazy(() => StreamStatusTerminalSchema), z.lazy(() => StreamStatusExitSchema), z.lazy(() => StreamStatusErrorSchema)]);
