// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ContainerSurface} from "../models/ContainerSurface.js";
import {ContainerSurfaceKindSchema} from "./ContainerSurfaceKind.js";

export const ContainerSurfaceSchema: z.ZodType<ContainerSurface> = z.object({
  "exposure_id": z.string().nullable().exactOptional(),
  "kind": z.lazy(() => ContainerSurfaceKindSchema),
  "port": z.int().min(0),
});
