// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentPackages} from "../models/EnvironmentPackages.js";

export const EnvironmentPackagesSchema: z.ZodType<EnvironmentPackages> = z.strictObject({
  "allows_package_manager": z.boolean().exactOptional(),
  "registries": z.array(z.string()).exactOptional(),
});
