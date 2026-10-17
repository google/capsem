// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ContainerStatusResponse} from "../models/ContainerStatusResponse.js";
import {ContainerStateSchema} from "./ContainerState.js";
import {ContainerSurfaceSchema} from "./ContainerSurface.js";

export const ContainerStatusResponseSchema: z.ZodType<ContainerStatusResponse> = z.object({
  "digest": z.string().nullable().exactOptional(),
  "error": z.string().nullable().exactOptional(),
  "exit_code": z.int().nullable().exactOptional(),
  "image": z.string(),
  "resolved": z.string().nullable().exactOptional(),
  "state": z.lazy(() => ContainerStateSchema),
  "surface": z.union([z.null(), z.lazy(() => ContainerSurfaceSchema)]).exactOptional(),
});
