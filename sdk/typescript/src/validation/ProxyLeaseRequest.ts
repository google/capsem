// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProxyLeaseRequest} from "../models/ProxyLeaseRequest.js";

export const ProxyLeaseRequestSchema: z.ZodType<ProxyLeaseRequest> = z.object({
  "lease_token": z.string(),
});
