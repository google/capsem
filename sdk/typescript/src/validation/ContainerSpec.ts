// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ContainerSpec} from "../models/ContainerSpec.js";
import {RegistryAccessSchema} from "./RegistryAccess.js";

export const ContainerSpecSchema: z.ZodType<ContainerSpec> = z.object({
  "args": z.array(z.string()).exactOptional(),
  "attach": z.boolean().exactOptional(),
  "env": z.record(z.string(), z.string()).exactOptional(),
  "image": z.string(),
  "registry": z.union([z.null(), z.lazy(() => RegistryAccessSchema)]).exactOptional(),
});
