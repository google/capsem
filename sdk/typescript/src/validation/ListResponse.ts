// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ListResponse} from "../models/ListResponse.js";
import {SandboxInfoSchema} from "./SandboxInfo.js";

export const ListResponseSchema: z.ZodType<ListResponse> = z.object({
  "sandboxes": z.array(z.lazy(() => SandboxInfoSchema)),
});
