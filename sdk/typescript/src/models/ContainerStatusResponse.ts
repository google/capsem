// Generated from Capsem OpenAPI. Do not edit.

import type { ContainerState } from "./ContainerState.js";
import type { ContainerSurface } from "./ContainerSurface.js";

export interface ContainerStatusResponse {
  "digest"?: string | null;
  "error"?: string | null;
  "exit_code"?: number | null;
  "image": string;
  "state": ContainerState;
  "surface"?: null | ContainerSurface;
}
