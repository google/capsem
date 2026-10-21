// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {StreamControl} from "../models/StreamControl.js";
import {StreamControlCloseStdinSchema} from "./StreamControlCloseStdin.js";
import {StreamControlFocusSchema} from "./StreamControlFocus.js";
import {StreamControlResizeSchema} from "./StreamControlResize.js";
import {StreamControlStartSchema} from "./StreamControlStart.js";

export const StreamControlSchema: z.ZodType<StreamControl> = z.union([z.lazy(() => StreamControlStartSchema), z.lazy(() => StreamControlResizeSchema), z.lazy(() => StreamControlFocusSchema), z.lazy(() => StreamControlCloseStdinSchema)]);
