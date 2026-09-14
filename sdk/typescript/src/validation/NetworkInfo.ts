// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {NetworkInfo} from "../models/NetworkInfo.js";
import {NetworkMemberInfoSchema} from "./NetworkMemberInfo.js";

export const NetworkInfoSchema: z.ZodType<NetworkInfo> = z.object({
  "created_unix_ms": z.int(),
  "id": z.string(),
  "members": z.array(z.lazy(() => NetworkMemberInfoSchema)),
  "name": z.string(),
});
