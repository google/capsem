// Generated from Capsem OpenAPI. Do not edit.

import type { InteractionBlock } from "./InteractionBlock.js";
import type { InteractionMessageKind } from "./InteractionMessageKind.js";
import type { InteractionRole } from "./InteractionRole.js";

export interface InteractionMessage {
  "blocks": Array<InteractionBlock>;
  "kind": InteractionMessageKind;
  "role": InteractionRole;
}
