// Generated from Capsem OpenAPI. Do not edit.

import type { StreamStatusError } from "./StreamStatusError.js";
import type { StreamStatusExit } from "./StreamStatusExit.js";
import type { StreamStatusStarted } from "./StreamStatusStarted.js";
import type { StreamStatusTerminal } from "./StreamStatusTerminal.js";

export type StreamStatus = StreamStatusStarted | StreamStatusTerminal | StreamStatusExit | StreamStatusError;
