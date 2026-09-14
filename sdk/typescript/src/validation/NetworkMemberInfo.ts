// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {NetworkMemberInfo} from "../models/NetworkMemberInfo.js";
import {NetworkMemberStateSchema} from "./NetworkMemberState.js";

export const NetworkMemberInfoSchema: z.ZodType<NetworkMemberInfo> = z.object({
  "address": z.ipv4(),
  "state": z.lazy(() => NetworkMemberStateSchema),
  "updated_unix_ms": z.int(),
  "vm_id": z.string(),
});
