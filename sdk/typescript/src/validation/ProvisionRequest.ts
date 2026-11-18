// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProvisionRequest} from "../models/ProvisionRequest.js";
import {ContainerSpecSchema} from "./ContainerSpec.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";
import {ManagedSessionCreateSchema} from "./ManagedSessionCreate.js";
import {VmLifecycleSelectionSchema} from "./VmLifecycleSelection.js";

export const ProvisionRequestSchema: z.ZodType<ProvisionRequest> = z.strictObject({
  "attach": z.boolean().exactOptional(),
  "container": z.union([z.null(), z.lazy(() => ContainerSpecSchema)]).exactOptional(),
  "cpus": z.int().min(0).nullable().exactOptional(),
  "env": z.record(z.string(), z.string()).nullable().exactOptional(),
  "environment": z.union([z.null(), z.lazy(() => EnvironmentRevisionReferenceSchema)]).exactOptional(),
  "expected_source": z.union([z.null(), z.lazy(() => VmLifecycleSelectionSchema)]).exactOptional(),
  "from": z.string().nullable().exactOptional(),
  "labels": z.record(z.string(), z.string()).nullable().exactOptional(),
  "managed": z.union([z.null(), z.lazy(() => ManagedSessionCreateSchema)]).exactOptional(),
  "name": z.string().nullable().exactOptional(),
  "networks": z.array(z.string()).exactOptional(),
  "persistent": z.boolean().exactOptional(),
  "ram_mb": z.int().min(0).nullable().exactOptional(),
});
