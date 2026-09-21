// Generated from Capsem OpenAPI. Do not edit.

import type { CapturedPayload } from "./CapturedPayload.js";
import type { InteractionBlockKind } from "./InteractionBlockKind.js";

export interface InteractionBlock {
  "kind": InteractionBlockKind;
  "payload"?: null | CapturedPayload;
}
