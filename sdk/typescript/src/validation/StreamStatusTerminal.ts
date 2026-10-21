// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {StreamStatusTerminal} from "../models/StreamStatusTerminal.js";
import {TerminalStateSchema} from "./TerminalState.js";

export const StreamStatusTerminalSchema: z.ZodType<StreamStatusTerminal> = z.object({
  "state": z.lazy(() => TerminalStateSchema),
  "type": z.literal("terminal"),
});
